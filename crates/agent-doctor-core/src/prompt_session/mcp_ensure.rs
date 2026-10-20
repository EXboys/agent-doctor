//! Ensure selected MCP servers are wired into the Ask runtime before spawn.
//!
//! Ask `@mcp:` used to be prompt-only. For Claude Code / Codex / Hermes / OpenClaw
//! we upsert the Agent Doctor browser MCP entry into the config paths those CLIs load.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::Context;

use crate::browser_wire::{
    ensure_chrome_for_wire, wire_browser_mcp, wire_computer_mcp, WireBrowserMcpOptions,
    BROWSER_MCP_WIRE_RUNTIMES,
};

use crate::workspace::{load_workspaces, resolve_agent_doctor_binary};

use super::PromptSessionOptions;

/// True when this Ask turn should expose browser MCP tools.
pub(crate) fn wants_browser_mcp(options: &PromptSessionOptions) -> bool {
    if options.browser_tools_enabled == Some(false) {
        return false;
    }
    if options
        .selected_mcps
        .iter()
        .any(|name| is_browser_mcp_name(name))
    {
        return true;
    }
    prompt_requests_browser_mcp(last_user_prompt_for_intent(&options.prompt))
}

/// Ask UI may prepend conversation history. Only the current user turn should
/// decide whether this turn needs browser MCP (otherwise a later "你好" still
/// injects BROWSER MCP REQUIRED and can hang on MCP startup).
pub(crate) fn last_user_prompt_for_intent(prompt: &str) -> &str {
    let prompt = prompt.trim();
    let rest = if let Some(idx) = prompt.rfind("\nUser: ") {
        prompt[idx + "\nUser: ".len()..].trim()
    } else if let Some(stripped) = prompt.strip_prefix("User: ") {
        stripped.trim()
    } else {
        prompt
    };
    rest.strip_suffix("\n\nAssistant:")
        .or_else(|| rest.strip_suffix("\nAssistant:"))
        .unwrap_or(rest)
        .trim()
}

pub(crate) fn prompt_requests_browser_mcp(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("browser mcp")
        || lower.contains("@mcp:browser")
        || lower.contains("mcp server: browser")
        || lower.contains("browser_navigate")
        || lower.contains("mcp__browser")
    {
        return true;
    }
    // Natural-language browser automation (Chinese / English).
    if prompt.contains("浏览器") {
        return true;
    }
    if lower.contains("open browser")
        || lower.contains("launch browser")
        || lower.contains("use the browser")
        || lower.contains("with the browser")
        || lower.contains("in the browser")
        || lower.contains("navigate to")
        || lower.contains("open the page")
        || lower.contains("open webpage")
        || lower.contains("open website")
    {
        return true;
    }
    // "open https://..." / "visit baidu.com" style intents.
    if (lower.contains("open ") || lower.contains("visit ") || lower.contains("go to "))
        && (lower.contains("http://")
            || lower.contains("https://")
            || lower.contains(".com")
            || lower.contains(".cn")
            || lower.contains("baidu")
            || lower.contains("google"))
    {
        return true;
    }
    false
}

/// Extra prompt block so the model prefers MCP browser tools over `open`/`curl`.
pub(crate) fn browser_mcp_tool_instructions() -> &'static str {
    "BROWSER MCP REQUIRED for this turn:\n\
     - Use the configured MCP server named `browser`.\n\
     - Workflow: `browser_navigate` → `browser_snapshot` → act with `@eN` refs → \
`browser_snapshot` again after navigation or DOM changes.\n\
     - Prefer `browser_click` / `browser_fill` with `ref` like `@e1` from snapshot \
(not guessed CSS). CSS `selector` is only a fallback.\n\
     - If refs are awkward, use `browser_find` with strategy \
role|label|text|placeholder|testid (optional action=click|fill).\n\
     - Wait with `browser_wait` load=networkidle when SPAs are still settling.\n\
     - Persist logins via `browser_state_save` / `browser_state_load` (path or session name).\n\
     - Also available: `browser_type`, `browser_screenshot` (returns PNG path by default; set inline=true only if you need base64), `browser_get_text`, \
`browser_list_tabs`.\n\
     - To open a website, call `browser_navigate` with the target URL \
(e.g. https://www.baidu.com), then `browser_snapshot`.\n\
     - NEVER use shell `open`, `xdg-open`, `osascript`, Safari/Chrome CLI, or curl/CDP hacks to browse.\n\
     - Old `@eN` refs become stale after page changes — always re-snapshot before the next click/fill."
}

/// The desktop switch is on. The model must see real tools, not a skill note.
pub(crate) fn wants_computer_use(options: &PromptSessionOptions) -> bool {
    options.computer_use_enabled == Some(true)
}

pub(crate) fn computer_use_tool_instructions() -> &'static str {
    "COMPUTER MCP REQUIRED for this turn:\n\
     - The app switch for controlling this computer is ON. You have an MCP server named `computer`.\n\
     - Its tools are `screen_apps`, `screen_open`, `screen_snapshot`, `screen_click`, `screen_type`, `screen_press`, `screen_scroll`, `screen_windows`, `screen_state`, `screen_set`, and `screen_menu`.\n\
     - Call the form that is already in your tool list. That is the bare name (`screen_snapshot`), `computer__screen_snapshot`, or `mcp__computer__screen_snapshot`. The same three forms apply to the other computer tools.\n\
     - The tool list may have changed since earlier turns. Earlier replies that said it was not connected are out of date. Do not decide from memory: call one of these tools now.\n\
     - Say 操作这台电脑还没接上 only after a call to one of these tools returns an error saying the tool does not exist. Do not offer other tasks.\n\
     - Do not say computer use is only a skill, and do not say you only have browser MCP.\n\
     - To open an app or switch to it, call `screen_open` with the exact name (`app`: 微信). This is the only way to open an app. Do not click the Dock, Launchpad, Spotlight, or a green icon. Do not press command+space or command+tab to open or switch apps. 微信 and 微信开发者工具 are different apps.\n\
     - After `screen_open`, snapshot, then Read the PNG, then click inside that window.\n\
     - If the row is not in the picture, call `screen_scroll` on that list (`direction`: down or up, `x` and `y` in points on the list). Do not stop just because the row is off screen.\n\
     - A row in the snapshot is `@eN` plus its name. Click that ref. Do not invent a nearby point for a row that already has `@eN`.\n\
     - One step, then look again. Snapshot, Read the PNG, then do the single next click, type, or key. After it returns, snapshot again before the next click. Click x,y are points inside that picture (divide image pixels by scale). If the picture is the app's window, click inside it. If a click says the picture did not change, snapshot and click once more. Do not type after that miss.\n\
     - To open a menu, call `screen_menu` with `path` from the menu bar down, such as [\"文件\", \"新建\"]. Do not guess a menu point from the picture.\n\
     - `screen_state` lists named buttons and text fields. `screen_set` writes a named text field from that list. If the window has no field, click the place and use `screen_type`. `screen_click` can take that `label` instead of x,y.\n\
     - `screen_type` takes `text`. `screen_press` is only for a key inside the app, such as return, escape, tab, backspace, or delete. backspace removes the character before the cursor. delete removes the character after the cursor, so it changes nothing when the cursor is already at the end of the field. It does not open or switch apps.\n\
     - Do not use Bash, `screencapture`, `osascript`, `cliclick`, or `open` to look at or click the screen.\n\
     - If `screen_snapshot` fails, say its error in one sentence and stop. Do not click, type, or press keys after that. A missing screenshot means you cannot see the screen.\n\
     - If a click says the settings page is already open, say that in one sentence and stop. Do not retry the click, and do not describe an Allow dialog that is not on screen. If control is already allowed, never ask for it again.\n\
     - Ask the user before sending a message, paying, or deleting."
}

/// The app already showed the user one sentence about the missing tools.
fn hand_missing_instructions(what: &str) -> String {
    format!(
        "{what} TOOLS ARE NOT CONNECTED this turn. The app already told the user why.\n\
         - Do not explain it again, and do not work around it with shell commands, screenshots, or scripts.\n\
         - If the request needs these tools, reply only 先按上面的提示处理，再发一次。 and stop."
    )
}

/// Status phase for a hand that could not be wired. The message is
/// `<hand>:<reason>`, then the original error on the next lines.
pub const HAND_MISSING_PHASE: &str = "hand_missing";

enum HandWire {
    Ready(String),
    Missing {
        reason: &'static str,
        detail: String,
    },
}

/// Which hands this turn has, after wiring them into the runtime's config.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Hands {
    pub browser: bool,
    pub computer: bool,
    browser_missing: bool,
    computer_missing: bool,
}

impl Hands {
    pub(crate) fn any(&self) -> bool {
        self.browser || self.computer || self.browser_missing || self.computer_missing
    }

    pub(crate) fn instructions(&self) -> String {
        let mut blocks: Vec<String> = Vec::new();
        if self.browser {
            blocks.push(browser_mcp_tool_instructions().to_string());
        }
        if self.browser_missing {
            blocks.push(hand_missing_instructions("BROWSER"));
        }
        if self.computer {
            blocks.push(computer_use_tool_instructions().to_string());
        }
        if self.computer_missing {
            blocks.push(hand_missing_instructions("COMPUTER"));
        }
        blocks.join("\n\n")
    }

    pub(crate) fn prompt(&self, prompt: &str) -> String {
        let prefix = self.instructions();
        if prefix.is_empty() {
            prompt.to_string()
        } else {
            format!("{prefix}\n\n{prompt}")
        }
    }
}

/// Wire the browser and computer tools this turn asked for. `status` gets
/// `(phase, message)`: quiet `info` when ready, [`HAND_MISSING_PHASE`] when not.
pub(crate) fn wire_hands(
    runtime: &str,
    options: &PromptSessionOptions,
    cwd: &Path,
    overlay: &HashMap<String, String>,
    mut status: impl FnMut(&'static str, String),
) -> Hands {
    let mut hands = Hands::default();
    let mut report = |hand: &str, wire: HandWire| -> bool {
        match wire {
            HandWire::Ready(note) => {
                if !note.is_empty() {
                    status("info", note);
                }
                true
            }
            HandWire::Missing { reason, detail } => {
                status(HAND_MISSING_PHASE, format!("{hand}:{reason}\n{detail}"));
                false
            }
        }
    };
    if wants_browser_mcp(options) {
        let ready = report("browser", ensure_browser_mcp_for_ask(runtime, cwd, overlay));
        hands.browser = ready;
        hands.browser_missing = !ready;
    }
    if wants_computer_use(options) {
        let ready = report(
            "computer",
            ensure_computer_mcp_for_ask(runtime, cwd, overlay),
        );
        hands.computer = ready;
        hands.computer_missing = !ready;
    }
    hands
}

fn wire_report(hand: &str, report: crate::browser_wire::BrowserMcpWireReport) -> HandWire {
    let mut ready = Vec::new();
    let mut failed = Vec::new();
    for item in report.results {
        if item.ok {
            ready.push(format!("{hand} MCP ready for {}", item.runtime));
        } else {
            failed.push(format!(
                "{hand} MCP wire failed for {}: {}",
                item.runtime, item.message
            ));
        }
    }
    if failed.is_empty() {
        HandWire::Ready(ready.join("; "))
    } else {
        HandWire::Missing {
            reason: "other",
            detail: failed.join("; "),
        }
    }
}

fn is_browser_mcp_name(name: &str) -> bool {
    let trimmed = name.trim().to_ascii_lowercase();
    trimmed == "browser" || trimmed.contains("browser")
}

pub(crate) fn runtime_supports_browser_mcp(runtime: &str) -> bool {
    BROWSER_MCP_WIRE_RUNTIMES
        .iter()
        .any(|id| id.eq_ignore_ascii_case(runtime.trim()))
}

/// Upsert browser MCP into Claude / Codex / Hermes / OpenClaw config paths.
///
/// Best-effort: failures are logged via the returned message but do not abort Ask.
fn ensure_browser_mcp_for_ask(
    runtime: &str,
    project_cwd: &Path,
    overlay: &HashMap<String, String>,
) -> HandWire {
    if !runtime_supports_browser_mcp(runtime) {
        return HandWire::Missing {
            reason: "runtime",
            detail: format!(
                "browser MCP is not wired for runtime '{runtime}' yet (supported: {})",
                BROWSER_MCP_WIRE_RUNTIMES.join(", ")
            ),
        };
    }

    if let Err(err) = ensure_chrome_for_wire() {
        return HandWire::Missing {
            reason: "chrome",
            detail: format!("browser MCP skipped: Chrome not found ({err})"),
        };
    }
    let binary = match resolve_mcp_binary() {
        Ok(path) => path,
        Err(err) => {
            return HandWire::Missing {
                reason: "cli",
                detail: format!("browser MCP skipped: {err}"),
            }
        }
    };

    let (project_path, codex_home, hermes_home, openclaw_workspace) =
        resolve_workspace_paths(project_cwd, overlay);
    let mut options = WireBrowserMcpOptions::with_binary(binary);
    options.project_path = project_path;
    options.codex_home = codex_home;
    options.hermes_home = hermes_home;
    options.openclaw_workspace = openclaw_workspace;
    options.runtimes = vec![runtime.to_string()];

    match wire_browser_mcp(&options) {
        Ok(report) => wire_report("browser", report),
        Err(err) => HandWire::Missing {
            reason: "other",
            detail: format!("browser MCP skipped: {err}"),
        },
    }
}

fn ensure_computer_mcp_for_ask(
    runtime: &str,
    project_cwd: &Path,
    overlay: &HashMap<String, String>,
) -> HandWire {
    if !runtime_supports_browser_mcp(runtime) {
        return HandWire::Missing {
            reason: "runtime",
            detail: format!(
                "computer MCP is not wired for runtime '{runtime}' yet (supported: {})",
                BROWSER_MCP_WIRE_RUNTIMES.join(", ")
            ),
        };
    }
    let binary = match resolve_computer_mcp_binary() {
        Ok(path) => path,
        Err(err) => {
            return HandWire::Missing {
                reason: "cli",
                detail: format!("computer MCP skipped: {err}"),
            }
        }
    };
    let (project_path, codex_home, hermes_home, openclaw_workspace) =
        resolve_workspace_paths(project_cwd, overlay);
    let mut options = WireBrowserMcpOptions::with_binary(binary);
    options.project_path = project_path;
    options.codex_home = codex_home;
    options.hermes_home = hermes_home;
    options.openclaw_workspace = openclaw_workspace;
    options.runtimes = vec![runtime.to_string()];
    match wire_computer_mcp(&options) {
        Ok(report) => wire_report("computer", report),
        Err(err) => HandWire::Missing {
            reason: "other",
            detail: format!("computer MCP skipped: {err}"),
        },
    }
}

fn resolve_workspace_paths(
    project_cwd: &Path,
    overlay: &HashMap<String, String>,
) -> (
    Option<PathBuf>,
    Option<PathBuf>,
    Option<PathBuf>,
    Option<PathBuf>,
) {
    let doc = load_workspaces().unwrap_or_default();
    let bound_name = overlay
        .get("AGENT_DOCTOR_WORKSPACE")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());
    let entry = bound_name
        .and_then(|name| doc.workspaces.get(name))
        .or_else(|| {
            doc.active
                .as_ref()
                .and_then(|name| doc.workspaces.get(name))
        });

    let project_path = entry
        .map(|e| e.path.clone())
        .or_else(|| Some(project_cwd.to_path_buf()));

    let codex_home = overlay
        .get("CODEX_HOME")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| entry.map(|e| e.codex_home.clone()));

    let hermes_home = overlay
        .get("HERMES_HOME")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| entry.map(|e| dirs_home().join(".hermes/profiles").join(&e.hermes_profile)));

    let openclaw_workspace = overlay
        .get("OPENCLAW_WORKSPACE")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| entry.map(|e| e.openclaw_workspace.clone()));

    (project_path, codex_home, hermes_home, openclaw_workspace)
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

type ComputerSupportKey = (String, Option<SystemTime>, u64);

fn computer_support_cache() -> &'static Mutex<HashMap<ComputerSupportKey, bool>> {
    static CACHE: OnceLock<Mutex<HashMap<ComputerSupportKey, bool>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn binary_supports_computer(path: &Path) -> bool {
    let meta = std::fs::metadata(path).ok();
    let key = meta.as_ref().map(|meta| {
        (
            path.to_string_lossy().into_owned(),
            meta.modified().ok(),
            meta.len(),
        )
    });
    if let Some(key) = &key {
        let cache = computer_support_cache()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(hit) = cache.get(key) {
            return *hit;
        }
    }
    let supports = binary_contains_screen_open(path);
    if let Some(key) = key {
        computer_support_cache()
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(key, supports);
    }
    supports
}

fn binary_contains_screen_open(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    bytes_have_screen_open(&bytes)
}

fn bytes_have_screen_open(bytes: &[u8]) -> bool {
    const TOOL: &[u8] = b"screen_open";
    bytes.windows(TOOL.len()).any(|window| window == TOOL)
}

/// The installed CLI can be the same version number and still lack `mcp computer`.
/// Use that CLI when it has the command; otherwise a newer binary next to this app.
/// A binary found beside the app is copied into the stable command path when it
/// is newer, so the managed copy does not stay on a build without the tools.
fn resolve_computer_mcp_binary() -> anyhow::Result<PathBuf> {
    if let Ok(path) = resolve_mcp_binary() {
        if binary_supports_computer(&path) {
            return Ok(path);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(std::path::Path::to_path_buf);
        for _ in 0..8 {
            let Some(current) = dir else {
                break;
            };
            let candidate = current.join(if cfg!(windows) {
                "agent-doctor.exe"
            } else {
                "agent-doctor"
            });
            if candidate.is_file() && binary_supports_computer(&candidate) {
                let stable = crate::workspace::ensure_stable_agent_doctor_cli(&candidate)
                    .unwrap_or_else(|_| candidate.clone());
                if binary_supports_computer(&stable) {
                    return Ok(stable);
                }
                return Ok(candidate);
            }
            dir = current.parent().map(std::path::Path::to_path_buf);
        }
    }
    anyhow::bail!("the installed Agent Doctor CLI does not include computer tools yet")
}

fn resolve_mcp_binary() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var("AGENT_DOCTOR_BIN") {
        let path = PathBuf::from(path.trim());
        if !path.as_os_str().is_empty() && path.exists() {
            return Ok(path);
        }
    }
    // Prefer app-bundled / real CLI over a stale PATH shim (common C-end trap).
    let resolved = resolve_agent_doctor_binary().context("resolve agent-doctor MCP binary")?;
    crate::workspace::ensure_stable_agent_doctor_cli(&resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_browser_selection_and_prompt() {
        let mut opts = PromptSessionOptions {
            runtime: "codex".into(),
            prompt: "hello".into(),
            cwd: None,
            timeout_sec: 30,
            dangerously_skip_permissions: false,
            full_auto: false,
            resume_thread_id: None,
            selected_mcps: vec!["browser".into()],
            image_paths: Vec::new(),
            workspace_name: None,
            provider_id: None,
            model: None,
            browser_tools_enabled: None,
            computer_use_enabled: None,
        };
        assert!(wants_browser_mcp(&opts));
        opts.selected_mcps.clear();
        assert!(!wants_browser_mcp(&opts));
        opts.prompt = "请用浏览器 MCP 打开页面".into();
        assert!(wants_browser_mcp(&opts));
        opts.prompt = "打开浏览器访问百度".into();
        assert!(wants_browser_mcp(&opts));
        opts.prompt = "open https://www.baidu.com".into();
        assert!(wants_browser_mcp(&opts));
        opts.prompt = "Conversation so far:\n\nUser: 打开浏览器访问百度\n\nAssistant: opened\n\nUser: 你好\n\nAssistant:".into();
        assert!(!wants_browser_mcp(&opts));
        assert_eq!(last_user_prompt_for_intent(&opts.prompt), "你好");
        opts.browser_tools_enabled = Some(false);
        opts.prompt = "打开浏览器访问百度".into();
        opts.selected_mcps = vec!["browser".into()];
        assert!(!wants_browser_mcp(&opts));
        assert!(!wants_computer_use(&opts));
        opts.computer_use_enabled = Some(true);
        assert!(wants_computer_use(&opts));
        let hands = Hands {
            computer: true,
            ..Hands::default()
        };
        let lined = hands.prompt("你好");
        assert!(lined.contains("server named `computer`"));
        assert!(lined.contains("screen_snapshot"));
        assert!(lined.contains("mcp__computer__screen_snapshot"));
        assert!(lined.contains("computer__screen_snapshot"));
        assert!(lined.contains("你好"));
        let missing = Hands {
            computer_missing: true,
            ..Hands::default()
        };
        let lined = missing.prompt("你好");
        assert!(lined.contains("COMPUTER TOOLS ARE NOT CONNECTED"));
        assert!(!lined.contains("server named `computer`"));
        assert_eq!(Hands::default().prompt("你好"), "你好");
        assert!(bytes_have_screen_open(
            b"\0tools/list\0screen_open\0screen_snapshot\0"
        ));
        assert!(!bytes_have_screen_open(
            b"\0tools/list\0screen_click\0screen_snapshot\0"
        ));
        assert!(!bytes_have_screen_open(b"screen_ope"));
    }
}
