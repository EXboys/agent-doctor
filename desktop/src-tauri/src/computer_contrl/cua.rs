//! Try the Cua Driver already on this computer. A missing driver, a driver
//! that cannot act, or a failed call falls through to this folder's own steps.
//!
//! Clicks, typing, keys, and screenshots stay on our side. Cua's pixels are
//! window-local, and ours are screen points. The driver is only asked to open
//! an app, list windows, or invoke a menu.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const ATTEMPT: Duration = Duration::from_secs(4);

static UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn running_apps() -> Option<Vec<String>> {
    let value = call("list_apps", &json!({}))?;
    running_names(&value)
}

/// Launch by bundle id when we have one, then bring that process forward.
/// False means the caller should open the app itself.
pub fn launch(bundle_id: Option<&str>, name: &str) -> bool {
    let mut args = serde_json::Map::new();
    if let Some(id) = bundle_id.map(str::trim).filter(|id| !id.is_empty()) {
        args.insert("bundle_id".into(), json!(id));
    } else if !name.trim().is_empty() {
        args.insert("name".into(), json!(name.trim()));
    } else {
        return false;
    }
    let Some(launched) = call("launch_app", &Value::Object(args)) else {
        return false;
    };
    let Some(pid) = launched
        .get("pid")
        .and_then(Value::as_i64)
        .filter(|pid| *pid > 0)
    else {
        return false;
    };
    call("bring_to_front", &json!({ "pid": pid })).is_some()
}

pub fn menu(app: &str, names: &[String]) -> bool {
    if names.is_empty() {
        return false;
    }
    let Some(pid) = pid_for(app) else {
        return false;
    };
    call("invoke_menu", &json!({ "pid": pid, "path": names })).is_some()
}

pub fn windows(app: &str) -> Option<Vec<Value>> {
    let args = match pid_for(app) {
        Some(pid) => json!({ "pid": pid }),
        None if app.trim().is_empty() => json!({}),
        None => return None,
    };
    let value = call("list_windows", &args)?;
    window_rows(&value)
}

fn pid_for(app: &str) -> Option<i64> {
    let value = call("list_apps", &json!({}))?;
    pid_named(&value, app)
}

fn call(tool: &str, args: &Value) -> Option<Value> {
    if UNAVAILABLE.load(Ordering::Relaxed) {
        return None;
    }
    let path = driver_path()?;
    let mut child = Command::new(path);
    child
        .args(["call", tool, &args.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = child.spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() >= ATTEMPT => {
                let _ = child.kill();
                // Waiting here blocks the hand if the driver ignores the kill.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(_) => return None,
        }
    }
    let output = child.wait_with_output().ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if driver_cannot_act(output.status.code(), &stderr) {
        UNAVAILABLE.store(true, Ordering::Relaxed);
        return None;
    }
    if !output.status.success() {
        return None;
    }
    json_stdout(&String::from_utf8_lossy(&output.stdout))
}

fn driver_path() -> Option<PathBuf> {
    let mut found = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        found.push(PathBuf::from(home).join(".local/bin/cua-driver"));
    }
    found.push(PathBuf::from(
        "/Applications/CuaDriver.app/Contents/MacOS/cua-driver",
    ));
    found.push(PathBuf::from("/opt/homebrew/bin/cua-driver"));
    found.push(PathBuf::from("/usr/local/bin/cua-driver"));
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        found.push(local.join("Programs/cua-driver/cua-driver.exe"));
        found.push(local.join("cua-driver/cua-driver.exe"));
    }
    found.into_iter().find(|path| path.is_file())
}

fn driver_cannot_act(code: Option<i32>, stderr: &str) -> bool {
    code == Some(75)
        || stderr.contains("permissions_pending")
        || stderr.contains("Cua Driver daemon is not running")
}

fn json_stdout(stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find(['{', '['])?;
    serde_json::from_str(&trimmed[start..]).ok()
}

fn running_names(value: &Value) -> Option<Vec<String>> {
    let apps = value.get("apps")?.as_array()?;
    if !apps.iter().any(|app| app.get("running").is_some()) {
        return None;
    }
    let mut names = Vec::new();
    for app in apps {
        if app.get("running").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(name) = app
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !names.iter().any(|have| have == name) {
            names.push(name.to_string());
        }
    }
    Some(names)
}

fn pid_named(value: &Value, want: &str) -> Option<i64> {
    let apps = value.get("apps")?.as_array()?;
    let want = want.trim();
    for app in apps {
        if app.get("running").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let Some(pid) = app
            .get("pid")
            .and_then(Value::as_i64)
            .filter(|pid| *pid > 0)
        else {
            continue;
        };
        if want.is_empty() {
            if app.get("active").and_then(Value::as_bool) == Some(true) {
                return Some(pid);
            }
            continue;
        }
        let name = app.get("name").and_then(Value::as_str).unwrap_or("");
        let bundle = app.get("bundle_id").and_then(Value::as_str).unwrap_or("");
        if name.eq_ignore_ascii_case(want) || bundle.eq_ignore_ascii_case(want) {
            return Some(pid);
        }
    }
    None
}

fn window_rows(value: &Value) -> Option<Vec<Value>> {
    let windows = value.get("windows")?.as_array()?;
    let mut rows = Vec::new();
    for window in windows {
        let title = window
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("");
        if title.is_empty() {
            continue;
        }
        let app = window
            .get("app")
            .or_else(|| window.get("owner"))
            .or_else(|| window.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        rows.push(json!({ "app": app, "title": title }));
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::{driver_cannot_act, pid_named, running_names, window_rows};
    use serde_json::json;

    #[test]
    fn only_running_apps_are_listed() {
        let value = json!({
            "apps": [
                { "name": "微信", "running": true, "pid": 10 },
                { "name": "计算器", "running": false, "pid": 0 }
            ]
        });
        assert_eq!(running_names(&value).unwrap(), vec!["微信".to_string()]);
        assert_eq!(pid_named(&value, "微信"), Some(10));
        assert!(running_names(&json!({ "apps": [{ "name": "微信" }] })).is_none());
    }

    #[test]
    fn a_pending_permission_is_not_usable() {
        assert!(driver_cannot_act(
            Some(75),
            "permissions_pending: macOS Accessibility"
        ));
        assert!(driver_cannot_act(
            Some(1),
            "Cua Driver daemon is not running on /tmp/cua.sock"
        ));
        assert!(!driver_cannot_act(Some(1), "unknown key"));
    }

    #[test]
    fn window_rows_keep_title_and_owner() {
        let rows = window_rows(&json!({
            "windows": [
                { "title": "聊天", "owner": "微信" },
                { "title": "  " }
            ]
        }))
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["title"], "聊天");
        assert_eq!(rows[0]["app"], "微信");
    }
}
