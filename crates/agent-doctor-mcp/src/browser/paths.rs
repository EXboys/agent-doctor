use std::path::PathBuf;

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Everyday browser profile (Google Chrome / Brave / Edge / Chromium).
///
/// This is the user-data-dir parent (contains `Default`, `Profile 1`, …), not
/// `…/Default` itself.
pub fn system_chrome_user_data_dir(binary: Option<&PathBuf>) -> PathBuf {
    let home = home_dir();
    let kind = binary
        .map(|p| p.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    if cfg!(target_os = "macos") {
        if kind.contains("brave") {
            return home.join("Library/Application Support/BraveSoftware/Brave-Browser");
        }
        if kind.contains("edge") {
            return home.join("Library/Application Support/Microsoft Edge");
        }
        if kind.contains("chromium") {
            return home.join("Library/Application Support/Chromium");
        }
        home.join("Library/Application Support/Google/Chrome")
    } else if cfg!(target_os = "linux") {
        if kind.contains("brave") {
            return home.join(".config/BraveSoftware/Brave-Browser");
        }
        if kind.contains("edge") || kind.contains("msedge") {
            return home.join(".config/microsoft-edge");
        }
        if kind.contains("chromium") {
            return home.join(".config/chromium");
        }
        home.join(".config/google-chrome")
    } else {
        if kind.contains("brave") {
            return home.join(r"AppData\Local\BraveSoftware\Brave-Browser\User Data");
        }
        if kind.contains("edge") || kind.contains("msedge") {
            return home.join(r"AppData\Local\Microsoft\Edge\User Data");
        }
        if kind.contains("chromium") {
            return home.join(r"AppData\Local\Chromium\User Data");
        }
        home.join(r"AppData\Local\Google\Chrome\User Data")
    }
}

/// Isolated Agent Doctor profile (no shared cookies/login with everyday Chrome).
pub fn isolated_chrome_user_data_dir() -> PathBuf {
    let home = home_dir();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/agent-doctor/chrome-cdp")
    } else if cfg!(target_os = "linux") {
        home.join(".config/agent-doctor/chrome-cdp")
    } else {
        home.join(r"AppData\Local\agent-doctor\chrome-cdp")
    }
}

/// Resolve profile dir: explicit arg → env → isolated automation profile.
///
/// Default is the Agent Doctor chrome-cdp dir so everyday Chrome is never
/// locked or duplicated. Pass an explicit path (or use desktop "日常 Chrome")
/// when you intentionally want the shared login profile.
pub fn resolve_user_data_dir(explicit: Option<&PathBuf>, binary: Option<&PathBuf>) -> PathBuf {
    let _ = binary;
    if let Some(path) = explicit {
        if !path.as_os_str().is_empty() {
            return path.clone();
        }
    }
    if let Ok(custom) = std::env::var("AGENT_DOCTOR_CHROME_USER_DATA_DIR") {
        let path = PathBuf::from(custom);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    isolated_chrome_user_data_dir()
}

/// Resolve profile directory: explicit → env → `Default`.
///
/// Without this, multi-profile Chrome shows the account picker on launch.
pub fn resolve_profile_directory(explicit: Option<&str>) -> String {
    if let Some(name) = explicit {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(from_env) = std::env::var("AGENT_DOCTOR_CHROME_PROFILE_DIRECTORY") {
        let trimmed = from_env.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "Default".to_string()
}
