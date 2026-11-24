#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::fs;
#[cfg(target_os = "macos")]
use std::fs::OpenOptions;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::io::Write;
#[cfg(target_os = "macos")]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::path::PathBuf;
use std::process::Command;
#[cfg(target_os = "macos")]
use std::process::Stdio;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use crate::evotown::load_evotown_config;
use crate::profile::agent_profile_path;
#[cfg(windows)]
use crate::profile::read_company_profile;
#[cfg(not(windows))]
use crate::profile::GATEWAY_URL_ENV;
#[cfg(windows)]
use crate::setup::anthropic_gateway_url_from_evotown_base;
#[cfg(not(windows))]
use crate::setup::evotown_agent_env_path;
use crate::setup::{write_company_profile_with_gateway, COMPANY_API_KEY_ENV, EVOTOWN_API_KEY_ENV};
#[cfg(not(windows))]
use crate::workspace::active_env_path;

use super::*;

pub(crate) fn open_in_terminal(
    runtime: &str,
    argv: &[&str],
    cwd: &Path,
    _prompt: Option<&str>,
) -> Result<OpenSessionReport> {
    if argv.is_empty() {
        bail!("empty terminal command for {runtime}");
    }
    let resolved = resolve_launch_argv(argv);
    let refs: Vec<&str> = resolved.iter().map(String::as_str).collect();
    let command_line = wrap_with_company_env(&shell_join(&refs));
    launch_system_terminal(cwd, &command_line)?;
    Ok(OpenSessionReport {
        runtime: runtime.into(),
        method: OpenSessionMethod::Terminal,
        cwd: cwd.display().to_string(),
        target: command_line.clone(),
        detail: format!(
            "Opened system terminal in {} running `{command_line}`.",
            cwd.display()
        ),
    })
}

pub(crate) fn resolve_launch_argv(argv: &[&str]) -> Vec<String> {
    let mut resolved: Vec<String> = argv.iter().map(|part| (*part).to_string()).collect();
    if let Some(bin) = resolved.first_mut() {
        if let Some(path) = crate::adapters::util::find_binary(bin) {
            *bin = path.to_string_lossy().into_owned();
        }
    }
    resolved
}

/// Prefix a shell command so Evotown / company API keys and workspace env are available.
/// Prefer sourcing env files (never inline secrets into the displayed command).
pub(crate) fn wrap_with_company_env(command: &str) -> String {
    let _ = ensure_profile_env_from_evotown();

    #[cfg(not(windows))]
    {
        let mut parts: Vec<String> = Vec::new();
        // Workspace first so CODEX_HOME / HERMES_HOME isolation applies.
        if let Ok(path) = active_env_path() {
            if path.exists() {
                parts.push(format!(". {}", shell_single_quote(&path.to_string_lossy())));
            }
        }
        if let Some(path) = agent_profile_path().filter(|path| path.exists()) {
            parts.push(format!(". {}", shell_single_quote(&path.to_string_lossy())));
        }
        if let Some(path) = evotown_agent_env_path().filter(|path| path.exists()) {
            parts.push(format!(". {}", shell_single_quote(&path.to_string_lossy())));
        }
        if parts.is_empty() {
            return command.to_string();
        }
        // Codex reads OPENAI_BASE_URL / OPENAI_API_KEY; Claude Code reads
        // ANTHROPIC_BASE_URL / ANTHROPIC_API_KEY. profile.env uses AGENT_DOCTOR_*.
        format!(
            "set -a && {} && set +a && {} && {command}",
            parts.join(" && "),
            unix_key_and_base_exports()
        )
    }

    #[cfg(windows)]
    {
        let profile = read_company_profile().ok().flatten();
        let api_key = profile
            .as_ref()
            .and_then(|p| p.api_key.clone())
            .filter(|key| !key.trim().is_empty())
            .or_else(|| {
                load_evotown_config()
                    .ok()
                    .map(|config| config.api_key)
                    .filter(|key| !key.trim().is_empty())
            });
        let gateway = profile
            .as_ref()
            .and_then(|p| p.gateway_url.clone())
            .filter(|url| !url.trim().is_empty());
        match (api_key, gateway) {
            (Some(key), Some(url)) => {
                let escaped = key.replace('\'', "''");
                let escaped_url = url.replace('\'', "''");
                let anthropic_url = anthropic_gateway_url_from_evotown_base(
                    &crate::setup::evotown_base_from_gateway(&url),
                );
                let escaped_anthropic = anthropic_url.replace('\'', "''");
                format!(
                    "$env:OPENAI_API_KEY='{escaped}'; $env:{EVOTOWN_API_KEY_ENV}='{escaped}'; $env:{COMPANY_API_KEY_ENV}='{escaped}'; $env:OPENAI_BASE_URL='{escaped_url}'; $env:ANTHROPIC_API_KEY='{escaped}'; $env:ANTHROPIC_BASE_URL='{escaped_anthropic}'; {command}"
                )
            }
            (Some(key), None) => {
                let escaped = key.replace('\'', "''");
                format!(
                    "$env:OPENAI_API_KEY='{escaped}'; $env:{EVOTOWN_API_KEY_ENV}='{escaped}'; $env:{COMPANY_API_KEY_ENV}='{escaped}'; $env:ANTHROPIC_API_KEY='{escaped}'; {command}"
                )
            }
            _ => command.to_string(),
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn unix_key_and_base_exports() -> String {
    format!(
        "export {COMPANY_API_KEY_ENV}=\"${{{COMPANY_API_KEY_ENV}:-${{EVOTOWN_API_KEY:-$OPENAI_API_KEY}}}}\" && \
export OPENAI_API_KEY=\"${{{COMPANY_API_KEY_ENV}:-${{EVOTOWN_API_KEY:-$OPENAI_API_KEY}}}}\" && \
export {EVOTOWN_API_KEY_ENV}=\"${{{EVOTOWN_API_KEY_ENV}:-${{{COMPANY_API_KEY_ENV}:-$OPENAI_API_KEY}}}}\" && \
export OPENAI_BASE_URL=\"${{{GATEWAY_URL_ENV}:-$OPENAI_BASE_URL}}\" && \
export ANTHROPIC_API_KEY=\"${{ANTHROPIC_API_KEY:-${{{COMPANY_API_KEY_ENV}:-${{EVOTOWN_API_KEY:-$OPENAI_API_KEY}}}}}}\" && \
_ad_ev=\"${{AGENT_DOCTOR_EVOTOWN_URL:-$EVOTOWN_URL}}\" && \
export ANTHROPIC_BASE_URL=\"${{ANTHROPIC_BASE_URL:-${{_ad_ev:+${{_ad_ev%/}}/api/gateway/anthropic}}}}\" && \
unset _ad_ev"
    )
}

/// If Evotown is configured but profile.env is missing, recreate it so Doctor/Codex share one key.
pub(crate) fn ensure_profile_env_from_evotown() -> anyhow::Result<()> {
    let Some(path) = agent_profile_path() else {
        return Ok(());
    };
    if path.exists() {
        return Ok(());
    }
    let config = load_evotown_config()?;
    let gateway = format!("{}/api/gateway/v1", config.base_url.trim_end_matches('/'));
    write_company_profile_with_gateway(&path, &gateway, &config.api_key, &config.base_url)?;
    Ok(())
}

pub fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if (url.starts_with("http://") || url.starts_with("https://"))
            && try_open_in_macos_everyday_browser(url)
        {
            return Ok(());
        }
        let status = Command::new("/usr/bin/open") // raw-spawn-ok
            .arg(url)
            .status()
            .context("failed to run `open` for deep link")?;
        if !status.success() {
            bail!("`open` exited with {status}");
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("xdg-open") // raw-spawn-ok
            .arg(url)
            .status()
            .context("failed to run `xdg-open` for deep link")?;
        if !status.success() {
            bail!("`xdg-open` exited with {status}");
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        // ShellExecute shows the window and keeps every `&` in the address.
        // `cmd /c start` does neither: CREATE_NO_WINDOW hides the browser it
        // starts, and an unquoted `&` cuts an OAuth URL before `redirect_uri`,
        // so the page stays on the signed-in account and never comes back.
        open_url_with_shell(url)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("deep link open is not supported on this platform");
    }
}

/// Ask the shell to show `url`. Returns as soon as the shell accepts it, so a
/// missing protocol handler cannot freeze the desktop on "Opening…".
#[cfg(target_os = "windows")]
fn open_url_with_shell(url: &str) -> Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut std::ffi::c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let operation = wide("open");
    let file = wide(url);
    // SW_SHOWNORMAL. A null owner is the desktop. The shell copies both strings.
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if code <= 32 {
        bail!("could not open {url}");
    }
    Ok(())
}

/// When Agent Doctor's isolated `chrome-cdp` process is running on macOS, plain
/// `/usr/bin/open <https-url>` gets routed by LaunchServices to whichever
/// `com.google.Chrome` instance registered earlier — often the empty `chrome-cdp`
/// profile instead of the user's signed-in everyday browser.
///
/// Target the PID recorded in the everyday browser's `SingletonLock` directly
/// via a `kInternetEventClass`/`kAEGetURL` AppleEvent (or launch the everyday
/// binary directly when unlocked) so sign-in pages always reuse the user's
/// existing browser session.
#[cfg(target_os = "macos")]
fn try_open_in_macos_everyday_browser(url: &str) -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    // Only bypass `/usr/bin/open` while Agent Doctor's isolated `chrome-cdp`
    // browser is actually running; otherwise LaunchServices opens the user's
    // default browser directly without needing AppleEvents / Automation TCC.
    if macos_singleton_lock_chromium_pid(
        &home.join("Library/Application Support/agent-doctor/chrome-cdp"),
    )
    .is_none()
    {
        return false;
    }
    let Some(bundle_id) = macos_default_https_bundle_id(&home) else {
        return false;
    };
    let Some((app_name, bin_name, user_data_rel)) = macos_chromium_spec_for_bundle_id(&bundle_id)
    else {
        return false;
    };

    let user_data_dir = home.join(user_data_rel);
    if let Some(pid) = macos_singleton_lock_chromium_pid(&user_data_dir) {
        return macos_send_open_url_to_pid(pid, url).is_ok();
    }

    // Everyday Chromium is not running yet while `chrome-cdp` is active:
    // launch the everyday browser binary directly against its default profile.
    for root in [PathBuf::from("/Applications"), home.join("Applications")] {
        let binary = root.join(app_name).join("Contents/MacOS").join(bin_name);
        if binary.is_file() {
            if let Ok(mut child) = Command::new(&binary) // raw-spawn-ok
                .arg(url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return true;
            }
        }
    }

    false
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_singleton_lock_chromium_pid(user_data_dir: &Path) -> Option<i32> {
    let target = fs::read_link(user_data_dir.join("SingletonLock")).ok()?;
    let pid = parse_singleton_lock_pid(&target.to_string_lossy())?;
    macos_pid_executable_path(pid)
        .is_some_and(|path| is_chromium_executable_path(&path))
        .then_some(pid)
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn is_chromium_executable_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("google chrome")
        || lower.contains("microsoft edge")
        || lower.contains("brave browser")
        || lower.contains("chromium")
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn parse_singleton_lock_pid(target: &str) -> Option<i32> {
    let (_, pid_str) = target.trim().rsplit_once('-')?;
    let pid = pid_str.parse::<i32>().ok()?;
    (pid > 0).then_some(pid)
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_chromium_spec_for_bundle_id(
    bundle_id: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    match bundle_id.trim().to_ascii_lowercase().as_str() {
        "com.google.chrome" => Some((
            "Google Chrome.app",
            "Google Chrome",
            "Library/Application Support/Google/Chrome",
        )),
        "com.google.chrome.canary" => Some((
            "Google Chrome Canary.app",
            "Google Chrome Canary",
            "Library/Application Support/Google/Chrome Canary",
        )),
        "com.google.chrome.beta" => Some((
            "Google Chrome Beta.app",
            "Google Chrome Beta",
            "Library/Application Support/Google/Chrome Beta",
        )),
        "com.google.chrome.dev" => Some((
            "Google Chrome Dev.app",
            "Google Chrome Dev",
            "Library/Application Support/Google/Chrome Dev",
        )),
        "com.microsoft.edgemac" => Some((
            "Microsoft Edge.app",
            "Microsoft Edge",
            "Library/Application Support/Microsoft Edge",
        )),
        "com.brave.browser" => Some((
            "Brave Browser.app",
            "Brave Browser",
            "Library/Application Support/BraveSoftware/Brave-Browser",
        )),
        "org.chromium.chromium" => Some((
            "Chromium.app",
            "Chromium",
            "Library/Application Support/Chromium",
        )),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn macos_default_https_bundle_id(home: &Path) -> Option<String> {
    let plist = home
        .join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    if !plist.is_file() {
        return None;
    }
    let output = Command::new("/usr/bin/plutil") // raw-spawn-ok
        .args(["-convert", "json", "-o", "-"])
        .arg(&plist)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let handlers = root.get("LSHandlers")?.as_array()?;
    for item in handlers {
        let scheme = item
            .get("LSHandlerURLScheme")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if scheme.eq_ignore_ascii_case("https") {
            if let Some(role) = item
                .get("LSHandlerRoleAll")
                .or_else(|| item.get("LSHandlerRoleViewer"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "-")
            {
                return Some(role.to_ascii_lowercase());
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_pid_executable_path(pid: i32) -> Option<String> {
    const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;
    extern "C" {
        fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, buffersize: u32) -> i32;
    }
    let mut buf = [0_u8; PROC_PIDPATHINFO_MAXSIZE];
    // SAFETY: `buf` is a valid writable buffer of `PROC_PIDPATHINFO_MAXSIZE` bytes.
    let len = unsafe {
        proc_pidpath(
            pid,
            buf.as_mut_ptr().cast(),
            PROC_PIDPATHINFO_MAXSIZE as u32,
        )
    };
    if len <= 0 {
        return None;
    }
    let len = (len as usize).min(PROC_PIDPATHINFO_MAXSIZE);
    Some(String::from_utf8_lossy(&buf[..len]).into_owned())
}

#[cfg(target_os = "macos")]
fn macos_send_open_url_to_pid(pid: i32, url: &str) -> Result<()> {
    #[repr(C, packed(2))]
    struct AEDesc {
        descriptor_type: u32,
        data_handle: *mut std::ffi::c_void,
    }

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn AECreateDesc(
            type_code: u32,
            data_ptr: *const std::ffi::c_void,
            data_size: isize,
            result: *mut AEDesc,
        ) -> i32;
        fn AEDisposeDesc(the_ae_desc: *mut AEDesc) -> i32;
        fn AECreateAppleEvent(
            the_ae_event_class: u32,
            the_ae_event_id: u32,
            target: *const AEDesc,
            return_id: i16,
            transaction_id: i32,
            result: *mut AEDesc,
        ) -> i32;
        fn AEPutParamPtr(
            the_apple_event: *mut AEDesc,
            the_ae_keyword: u32,
            type_code: u32,
            data_ptr: *const std::ffi::c_void,
            data_size: isize,
        ) -> i32;
        fn AEGetParamPtr(
            the_apple_event: *const AEDesc,
            the_ae_keyword: u32,
            desired_type: u32,
            actual_type: *mut u32,
            data_ptr: *mut std::ffi::c_void,
            maximum_size: isize,
            actual_size: *mut isize,
        ) -> i32;
        fn AESendMessage(
            event: *const AEDesc,
            reply: *mut AEDesc,
            send_mode: i32,
            time_out_in_ticks: isize,
        ) -> i32;
    }

    const TYPE_KERNEL_PROCESS_ID: u32 = 0x6b70_6964; // 'kpid'
    const K_INTERNET_EVENT_CLASS: u32 = 0x4755_524c; // 'GURL'
    const K_AE_GET_URL: u32 = 0x4755_524c; // 'GURL'
    const K_AE_MISC_STANDARDS: u32 = 0x6d69_7363; // 'misc'
    const K_AE_ACTIVATE: u32 = 0x6163_7476; // 'actv'
    const KEY_DIRECT_OBJECT: u32 = 0x2d2d_2d2d; // '----'
    const KEY_ERROR_NUMBER: u32 = 0x6572_726e; // 'errn'
    const TYPE_UTF8_TEXT: u32 = 0x7574_6638; // 'utf8'
    const TYPE_SINT32: u32 = 0x6c6f_6e67; // 'long'
    const K_AUTO_GENERATE_RETURN_ID: i16 = -1;
    const K_ANY_TRANSACTION_ID: i32 = 0;
    const K_AE_NO_REPLY: i32 = 0x1;
    const K_AE_WAIT_REPLY: i32 = 0x3;
    const K_AE_DEFAULT_TIMEOUT: isize = -1;
    const WAIT_REPLY_TIMEOUT_TICKS: isize = 120; // 2 seconds

    // SAFETY: All descriptors are zero-initialized, checked for `noErr` (0), and disposed before return.
    unsafe {
        let mut target = AEDesc {
            descriptor_type: 0,
            data_handle: std::ptr::null_mut(),
        };
        let err = AECreateDesc(
            TYPE_KERNEL_PROCESS_ID,
            (&pid as *const i32).cast(),
            std::mem::size_of::<i32>() as isize,
            &mut target,
        );
        if err != 0 {
            bail!("AECreateDesc failed with OSStatus {err}");
        }

        let mut gurl_event = AEDesc {
            descriptor_type: 0,
            data_handle: std::ptr::null_mut(),
        };
        let create_err = AECreateAppleEvent(
            K_INTERNET_EVENT_CLASS,
            K_AE_GET_URL,
            &target,
            K_AUTO_GENERATE_RETURN_ID,
            K_ANY_TRANSACTION_ID,
            &mut gurl_event,
        );
        if create_err != 0 {
            let _ = AEDisposeDesc(&mut target);
            bail!("AECreateAppleEvent(GURL) failed with OSStatus {create_err}");
        }

        let bytes = url.as_bytes();
        let param_err = AEPutParamPtr(
            &mut gurl_event,
            KEY_DIRECT_OBJECT,
            TYPE_UTF8_TEXT,
            bytes.as_ptr().cast(),
            bytes.len() as isize,
        );
        let send_err = if param_err == 0 {
            let mut reply = AEDesc {
                descriptor_type: 0,
                data_handle: std::ptr::null_mut(),
            };
            // Wait for the reply so TCC Automation denials (-1743) or dead
            // targets surface as errors and fall back to `/usr/bin/open`
            // instead of silently swallowing the login URL.
            let status = AESendMessage(
                &gurl_event,
                &mut reply,
                K_AE_WAIT_REPLY,
                WAIT_REPLY_TIMEOUT_TICKS,
            );
            let mut handler_err: i32 = 0;
            let mut actual_type: u32 = 0;
            let mut actual_size: isize = 0;
            if status == 0
                && AEGetParamPtr(
                    &reply,
                    KEY_ERROR_NUMBER,
                    TYPE_SINT32,
                    &mut actual_type,
                    (&mut handler_err as *mut i32).cast(),
                    std::mem::size_of::<i32>() as isize,
                    &mut actual_size,
                ) == 0
                && handler_err != 0
            {
                let _ = AEDisposeDesc(&mut reply);
                handler_err
            } else {
                let _ = AEDisposeDesc(&mut reply);
                status
            }
        } else {
            param_err
        };
        let _ = AEDisposeDesc(&mut gurl_event);

        // Best-effort bring the everyday browser window to the front.
        if send_err == 0 {
            let mut actv_event = AEDesc {
                descriptor_type: 0,
                data_handle: std::ptr::null_mut(),
            };
            if AECreateAppleEvent(
                K_AE_MISC_STANDARDS,
                K_AE_ACTIVATE,
                &target,
                K_AUTO_GENERATE_RETURN_ID,
                K_ANY_TRANSACTION_ID,
                &mut actv_event,
            ) == 0
            {
                let mut reply = AEDesc {
                    descriptor_type: 0,
                    data_handle: std::ptr::null_mut(),
                };
                let _ = AESendMessage(&actv_event, &mut reply, K_AE_NO_REPLY, K_AE_DEFAULT_TIMEOUT);
                let _ = AEDisposeDesc(&mut actv_event);
            }
        }
        let _ = AEDisposeDesc(&mut target);

        if send_err != 0 {
            bail!("AESendMessage(GURL) failed with OSStatus {send_err}");
        }
    }
    Ok(())
}

pub(crate) fn launch_system_terminal(cwd: &Path, command_line: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // Prefer `open … .command` over AppleScript:
        // 1) Launch Services brings Terminal.app to the front (Ask window stays on top
        //    otherwise, so users think "打开终端" did nothing).
        // 2) No Automation permission is required to control Terminal.
        // Terminal.app's AppleScript `do script` also truncates long strings (~1024
        // bytes); the self-deleting script keeps Codex overrides intact.
        let launch_script = write_macos_terminal_launch_script(cwd, command_line)?;
        if Command::new("open") // raw-spawn-ok
            .arg(&launch_script)
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            // Nudge Terminal forward in case Ask / another always-focused window
            // kept covering the new tab.
            let _ = Command::new("open").args(["-a", "Terminal"]).status(); // raw-spawn-ok
            return Ok(());
        }

        // Fallback: AppleScript + activate (needs Automation permission).
        let invoke_script = shell_single_quote(&launch_script.to_string_lossy());
        let script = format!(
            "tell application \"Terminal\"\nactivate\ndo script \"{cmd}\"\nend tell",
            cmd = escape_applescript(&invoke_script),
        );
        // raw-spawn-ok
        let status = match Command::new("osascript").args(["-e", &script]).status() {
            Ok(status) => status,
            Err(error) => {
                let _ = fs::remove_file(&launch_script);
                return Err(error).context("failed to launch Terminal.app");
            }
        };
        if !status.success() {
            let _ = fs::remove_file(&launch_script);
            bail!("osascript exited with {status}");
        }
        let _ = Command::new("open").args(["-a", "Terminal"]).status(); // raw-spawn-ok
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let cwd_str = cwd.to_string_lossy();
        let shell_cmd = format!(
            "cd {} && exec {}",
            shell_single_quote(&cwd_str),
            command_line
        );
        let candidates: &[(&str, &[&str])] = &[
            ("x-terminal-emulator", &["-e", "bash", "-lc"]),
            ("gnome-terminal", &["--", "bash", "-lc"]),
            ("konsole", &["-e", "bash", "-lc"]),
            ("xterm", &["-e", "bash", "-lc"]),
        ];
        for (bin, prefix) in candidates {
            let mut cmd = Command::new(bin); // raw-spawn-ok
            cmd.args(*prefix).arg(&shell_cmd);
            if cmd.spawn().is_ok() {
                return Ok(());
            }
        }
        bail!("no known terminal emulator found (tried x-terminal-emulator, gnome-terminal, konsole, xterm)");
    }
    #[cfg(target_os = "windows")]
    {
        launch_windows_terminal(cwd, command_line)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (cwd, command_line);
        bail!("system terminal launch is not supported on this platform");
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn write_macos_terminal_launch_script(
    cwd: &Path,
    command_line: &str,
) -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir();

    for attempt in 0..16 {
        // `.command` is opened by Terminal.app via Launch Services (`open`), which
        // focuses the terminal — unlike a bare `.sh` handed only to AppleScript.
        let path = dir.join(format!(
            "agent-doctor-terminal-{}-{nonce}-{attempt}.command",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path);
        let mut file = match file {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).context("failed to create terminal launch script");
            }
        };

        let script_path = shell_single_quote(&path.to_string_lossy());
        let cwd = shell_single_quote(&cwd.to_string_lossy());
        if let Err(error) = writeln!(
            file,
            "#!/bin/sh\nrm -f -- {script_path}\ncd {cwd} && {command_line}"
        ) {
            let _ = fs::remove_file(&path);
            return Err(error).context("failed to write terminal launch script");
        }
        return Ok(path);
    }

    bail!("failed to allocate a unique terminal launch script")
}

/// GUI apps must not wait on a console process, and must not pass `$env:…; cmd`
/// through `wt` / `cmd /C` — both treat `;` as extra windows.
#[cfg(target_os = "windows")]
pub(crate) fn launch_windows_terminal(cwd: &Path, command_line: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x00000010;

    let script = write_windows_terminal_launch_script(cwd, command_line)?;
    let script_arg = script.to_string_lossy().into_owned();
    let powershell = windows_powershell_exe();
    let mut cmd = Command::new(&powershell); // raw-spawn-ok
    cmd.args([
        "-NoLogo",
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        &script_arg,
    ])
    .creation_flags(CREATE_NEW_CONSOLE);
    match cmd.spawn() {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&script);
            Err(error).context("failed to launch PowerShell")
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_powershell_exe() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn render_windows_terminal_script(
    script_path: &str,
    cwd: &str,
    command_line: &str,
) -> String {
    format!(
        "Remove-Item -LiteralPath '{script_path}' -Force -ErrorAction SilentlyContinue\r\n\
         Set-Location -LiteralPath '{cwd}'\r\n\
         Write-Host 'Agent Doctor: starting session...'\r\n\
         Write-Host ''\r\n\
         $script:adFailed = $false\r\n\
         try {{\r\n\
         {command_line}\r\n\
         if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {{ $script:adFailed = $true }}\r\n\
         }} catch {{\r\n\
         Write-Host $_\r\n\
         $script:adFailed = $true\r\n\
         }}\r\n\
         Write-Host ''\r\n\
         if ($script:adFailed) {{\r\n\
         Write-Host 'The command exited with an error. Press Enter to close this window.'\r\n\
         }} else {{\r\n\
         Write-Host 'Session ended. Press Enter to close this window.'\r\n\
         }}\r\n\
         [void][System.Console]::ReadLine()\r\n"
    )
}

#[cfg(target_os = "windows")]
pub(crate) fn write_windows_terminal_launch_script(
    cwd: &Path,
    command_line: &str,
) -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir();
    let cwd = cwd.to_string_lossy().replace('\'', "''");
    for attempt in 0..16 {
        let path = dir.join(format!(
            "agent-doctor-terminal-{}-{nonce}-{attempt}.ps1",
            std::process::id()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let script_path = path.to_string_lossy().replace('\'', "''");
                if let Err(error) = write!(
                    file,
                    "{}",
                    render_windows_terminal_script(&script_path, &cwd, command_line)
                ) {
                    let _ = fs::remove_file(&path);
                    return Err(error).context("failed to write terminal launch script");
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).context("failed to create terminal launch script");
            }
        }
    }
    bail!("failed to allocate a unique terminal launch script")
}

pub(crate) fn shell_join(argv: &[&str]) -> String {
    #[cfg(windows)]
    {
        argv.iter()
            .enumerate()
            .map(|(index, part)| {
                if index == 0 {
                    let lower = part.to_ascii_lowercase();
                    if lower.ends_with(".cmd")
                        || lower.ends_with(".bat")
                        || part.contains(char::is_whitespace)
                    {
                        return format!("& '{}'", part.replace('\'', "''"));
                    }
                    return (*part).to_string();
                }
                if part.is_empty() || part.contains(char::is_whitespace) || part.contains('\'') {
                    format!("'{}'", part.replace('\'', "''"))
                } else {
                    (*part).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    #[cfg(not(windows))]
    {
        argv.iter()
            .map(|part| {
                if part.is_empty()
                    || part.contains(|c: char| c.is_whitespace() || "\"'\\$`".contains(c))
                {
                    shell_single_quote(part)
                } else {
                    (*part).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(not(windows))]
pub(crate) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
pub(crate) fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
