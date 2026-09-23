use std::path::PathBuf;
use std::process::Child;

/// Information about a discovered Chrome installation.
#[derive(Debug, Clone)]
pub struct BrowserDiscovery {
    pub binary_path: PathBuf,
    pub user_data_dir: PathBuf,
    /// Chrome profile directory name inside user-data-dir (`Default`, `Profile 2`, …).
    pub profile_directory: String,
    pub version: Option<String>,
}

/// A running Chrome instance controlled by agent-doctor.
#[derive(Debug)]
pub struct ChromeInstance {
    pub process: Option<Child>,
    pub debug_port: u16,
    pub user_data_dir: PathBuf,
    pub ws_endpoint: Option<String>,
}

/// Which Chromium-family browser to prefer when discovering a binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserFamily {
    /// Prefer Chrome, then Edge, then Chromium/Brave.
    #[default]
    Auto,
    Chrome,
    Edge,
    Chromium,
}

impl BrowserFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Chromium => "chromium",
        }
    }
}
