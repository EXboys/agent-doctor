//! Pick download hosts that work in mainland China.
//!
//! GitHub and registry.npmjs.org are often slow or blocked there. Detection uses
//! the machine timezone (not the app language). `AGENT_DOCTOR_DOWNLOAD_ROUTE`
//! overrides it: `china`, `official`, or `auto`.

use std::process::Command;

#[cfg(test)]
use std::ffi::OsStr;

pub const NPM_MIRROR: &str = "https://registry.npmmirror.com";
pub const NODE_MIRROR: &str = "https://cdn.npmmirror.com/binaries/node";
/// The only GitHub prefix that actually returned pack data from mainland China.
/// Others (ghfast, mirror.ghproxy, gh.ddlc) failed or refused the connection.
pub const GITHUB_MIRROR_PREFIX: &str = "https://gh-proxy.com/https://github.com/";

pub fn use_china_mirrors() -> bool {
    match std::env::var("AGENT_DOCTOR_DOWNLOAD_ROUTE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "china" | "cn" | "mirror" => return true,
        "official" | "github" | "off" => return false,
        _ => {}
    }
    timezone_is_mainland_china()
}

pub fn apply_china_download_env(command: &mut Command) {
    if use_china_mirrors() {
        apply_download_env(command);
    }
}

pub(crate) fn apply_download_env(command: &mut Command) {
    command.env("npm_config_registry", NPM_MIRROR);
    // Rewrite https://github.com/ clones for this process only. Does not touch
    // the user's git config. A stalled proxy is aborted so the installer can
    // fall back to GitHub itself instead of sitting on an open connection.
    command.env("GIT_CONFIG_COUNT", "3");
    command.env(
        "GIT_CONFIG_KEY_0",
        format!("url.{GITHUB_MIRROR_PREFIX}.insteadOf"),
    );
    command.env("GIT_CONFIG_VALUE_0", "https://github.com/");
    command.env("GIT_CONFIG_KEY_1", "http.lowSpeedLimit");
    command.env("GIT_CONFIG_VALUE_1", "1000");
    command.env("GIT_CONFIG_KEY_2", "http.lowSpeedTime");
    command.env("GIT_CONFIG_VALUE_2", "20");
}

fn timezone_is_mainland_china() -> bool {
    system_timezone().is_some_and(|tz| timezone_name_is_mainland(&tz))
}

pub(crate) fn timezone_name_is_mainland(tz: &str) -> bool {
    matches!(
        tz,
        "Asia/Shanghai" | "Asia/Chongqing" | "Asia/Harbin" | "Asia/Urumqi" | "Asia/Kashgar"
    ) || tz.eq_ignore_ascii_case("China Standard Time")
}

fn system_timezone() -> Option<String> {
    if let Some(tz) = platform_timezone() {
        return Some(tz);
    }
    std::env::var("TZ")
        .ok()
        .map(|tz| tz.trim().to_string())
        .filter(|tz| !tz.is_empty())
}

#[cfg(unix)]
fn platform_timezone() -> Option<String> {
    if let Some(tz) = timezone_from_localtime() {
        return Some(tz);
    }
    let text = std::fs::read_to_string("/etc/timezone").ok()?;
    let tz = text.trim();
    if tz.is_empty() {
        None
    } else {
        Some(tz.to_string())
    }
}

#[cfg(unix)]
fn timezone_from_localtime() -> Option<String> {
    let link = std::fs::read_link("/etc/localtime").ok()?;
    let text = link.to_string_lossy();
    let marker = "zoneinfo/";
    let start = text.find(marker)? + marker.len();
    let tz = text[start..].trim_matches('/').to_string();
    if tz.is_empty() {
        None
    } else {
        Some(tz)
    }
}

#[cfg(windows)]
fn platform_timezone() -> Option<String> {
    let output = Command::new("tzutil").arg("/g").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout);
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_timezone() -> Option<String> {
    None
}

#[cfg(test)]
fn command_has_env(command: &Command, key: &str, value: &str) -> bool {
    command
        .get_envs()
        .any(|(k, v)| k == OsStr::new(key) && v == Some(OsStr::new(value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_download_env_points_npm_and_git_at_mirrors() {
        let mut command = Command::new("true");
        apply_download_env(&mut command);
        assert!(command_has_env(&command, "npm_config_registry", NPM_MIRROR));
        assert!(command_has_env(
            &command,
            "GIT_CONFIG_KEY_0",
            "url.https://gh-proxy.com/https://github.com/.insteadOf"
        ));
        assert!(command_has_env(&command, "GIT_CONFIG_COUNT", "3"));
        assert!(command_has_env(&command, "GIT_CONFIG_VALUE_1", "1000"));
        assert!(command_has_env(&command, "GIT_CONFIG_VALUE_2", "20"));
    }

    #[test]
    fn mainland_timezones_are_recognized() {
        for tz in [
            "Asia/Shanghai",
            "Asia/Urumqi",
            "Asia/Chongqing",
            "China Standard Time",
        ] {
            assert!(timezone_name_is_mainland(tz), "{tz}");
        }
        assert!(!timezone_name_is_mainland("Asia/Hong_Kong"));
        assert!(!timezone_name_is_mainland("Asia/Taipei"));
        assert!(!timezone_name_is_mainland("America/Los_Angeles"));
    }
}
