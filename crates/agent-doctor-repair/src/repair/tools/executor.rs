use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::diff::unified_diff;
use super::discover::{grep_allowed_files, list_allowed_directory};
use super::patch::patch_structured_file;
use super::policy::{allowed_paths_for_runtime, bash_command_allowed, path_is_allowed};
use crate::repair::browser_tools_allowed;
use agent_doctor_core::mask::{is_env_file, mask_config_file, unmask_file_content, SecretVault};
use agent_doctor_core::repair_types::{RepairToolKind, RepairToolResult};
use agent_doctor_mcp::{call_browser_tool, call_computer_tool, SharedBrowser};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowserToolArgs {
    pub url: Option<String>,
    pub text: Option<String>,
    /// Snapshot ref (`@eN`) or CSS selector: click/fill target, snapshot scope.
    pub target: Option<String>,
    pub strategy: Option<String>,
    pub query: Option<String>,
    pub action: Option<String>,
    /// Screenshot output path; empty lets the browser pick a temp PNG.
    pub path: Option<String>,
    pub inline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairToolCall {
    pub kind: RepairToolKind,
    pub path: Option<String>,
    pub content: Option<String>,
    pub old_string: Option<String>,
    pub new_string: Option<String>,
    pub key_path: Option<String>,
    pub value: Option<String>,
    pub pattern: Option<String>,
    pub command: Option<String>,
    #[serde(default)]
    pub browser: Option<BrowserToolArgs>,
    /// One `screen_*` computer-use call: the tool name plus its raw JSON
    /// arguments, passed through to the computer server untouched.
    #[serde(default)]
    pub computer: Option<ComputerToolArgs>,
}

/// The exact `screen_*` tool invoked this turn and its arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputerToolArgs {
    pub tool: String,
    pub arguments: Value,
}

pub struct RepairToolExecutor {
    runtime_id: String,
    allowed_paths: Vec<PathBuf>,
    vault: SecretVault,
    apply_writes: bool,
    /// Browser session for the chat's browser tools. The chat shares one
    /// process-wide session (`Arc`), so Chrome outlives single turns. `None`
    /// keeps those calls failing cleanly (repair planner, deep diagnose).
    browser: Option<Arc<SharedBrowser>>,
    /// Computer-use tools need no session (every call reaches the desktop
    /// app over its socket), but they stay off unless the chat turn enabled
    /// them, so the repair planner can never click the real screen.
    screen_enabled: bool,
}

impl RepairToolExecutor {
    pub fn new(runtime_id: &str, vault: SecretVault, apply_writes: bool) -> Self {
        Self {
            runtime_id: runtime_id.to_string(),
            allowed_paths: allowed_paths_for_runtime(runtime_id),
            vault,
            apply_writes,
            browser: None,
            screen_enabled: false,
        }
    }

    pub fn with_allowed_paths(
        runtime_id: &str,
        vault: SecretVault,
        apply_writes: bool,
        allowed_paths: Vec<PathBuf>,
    ) -> Self {
        Self {
            runtime_id: runtime_id.to_string(),
            allowed_paths,
            vault,
            apply_writes,
            browser: None,
            screen_enabled: false,
        }
    }

    /// Attach the shared browser session (chat turns with browser tools on).
    pub fn set_browser(&mut self, browser: Arc<SharedBrowser>) {
        self.browser = Some(browser);
    }

    /// Allow `screen_*` computer-use calls on this executor (chat turns with
    /// the computer-use switch on).
    pub fn set_screen_enabled(&mut self, enabled: bool) {
        self.screen_enabled = enabled;
    }

    pub fn vault(&self) -> &SecretVault {
        &self.vault
    }

    pub fn vault_mut(&mut self) -> &mut SecretVault {
        &mut self.vault
    }

    pub fn into_vault(self) -> SecretVault {
        self.vault
    }

    pub fn execute(&mut self, call: &RepairToolCall) -> Result<RepairToolResult> {
        match call.kind {
            RepairToolKind::Read => self.read_file(call.path.as_deref()),
            RepairToolKind::ListDir => self.list_dir(call.path.as_deref()),
            RepairToolKind::GrepFiles => {
                self.grep_files(call.path.as_deref(), call.pattern.as_deref())
            }
            RepairToolKind::WriteFile => {
                self.write_file(call.path.as_deref(), call.content.as_deref())
            }
            RepairToolKind::SearchReplace => self.search_replace(
                call.path.as_deref(),
                call.old_string.as_deref(),
                call.new_string.as_deref(),
            ),
            RepairToolKind::PatchConfig => self.patch_config(
                call.path.as_deref(),
                call.key_path.as_deref(),
                call.value.as_deref(),
            ),
            RepairToolKind::Bash => self.run_bash(call.command.as_deref()),
            RepairToolKind::BrowserNavigate
            | RepairToolKind::BrowserSnapshot
            | RepairToolKind::BrowserClick
            | RepairToolKind::BrowserFill
            | RepairToolKind::BrowserFind
            | RepairToolKind::BrowserScreenshot
            | RepairToolKind::BrowserGetText => self.run_browser(call.kind, call),
            RepairToolKind::Screen => self.run_computer(call),
        }
    }

    fn read_file(&mut self, path: Option<&str>) -> Result<RepairToolResult> {
        let path = match self.resolve_path(path, RepairToolKind::Read) {
            Ok(path) => path,
            Err(result) => return Ok(result),
        };

        if !path.exists() {
            return Ok(ok(
                RepairToolKind::Read,
                false,
                None,
                format!("(file not found: {})", path.display()),
            ));
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut snippet_vault = SecretVault::default();
        let masked = mask_config_file(&path, &raw, &mut snippet_vault);
        self.vault.absorb(snippet_vault);

        Ok(ok(
            RepairToolKind::Read,
            false,
            None,
            format_numbered_lines(&masked),
        ))
    }

    fn list_dir(&mut self, path: Option<&str>) -> Result<RepairToolResult> {
        if let Some(path) = path {
            let path = PathBuf::from(path);
            if !path_is_allowed(&path, &self.allowed_paths) {
                return Ok(failed(
                    RepairToolKind::ListDir,
                    false,
                    format!("path not allowed: {}", path.display()),
                ));
            }
        }

        match list_allowed_directory(path.map(Path::new), &self.allowed_paths) {
            Ok(output) => Ok(ok(RepairToolKind::ListDir, false, None, output)),
            Err(error) => Ok(failed(RepairToolKind::ListDir, false, error.to_string())),
        }
    }

    fn grep_files(
        &mut self,
        path: Option<&str>,
        pattern: Option<&str>,
    ) -> Result<RepairToolResult> {
        let Some(pattern) = pattern else {
            return Ok(failed(
                RepairToolKind::GrepFiles,
                false,
                "grep_files requires pattern".to_string(),
            ));
        };

        let path_buf = path.map(PathBuf::from);
        if let Some(ref path) = path_buf {
            if !path_is_allowed(path, &self.allowed_paths) {
                return Ok(failed(
                    RepairToolKind::GrepFiles,
                    false,
                    format!("path not allowed: {}", path.display()),
                ));
            }
        }

        match grep_allowed_files(
            pattern,
            path_buf.as_deref(),
            &self.allowed_paths,
            &mut self.vault,
        ) {
            Ok(output) => Ok(ok(RepairToolKind::GrepFiles, false, None, output)),
            Err(error) => Ok(failed(RepairToolKind::GrepFiles, false, error.to_string())),
        }
    }

    fn write_file(
        &mut self,
        path: Option<&str>,
        content: Option<&str>,
    ) -> Result<RepairToolResult> {
        let path = match self.resolve_path(path, RepairToolKind::WriteFile) {
            Ok(path) => path,
            Err(result) => return Ok(result),
        };
        let Some(content) = content else {
            return Ok(failed(
                RepairToolKind::WriteFile,
                false,
                "write_file requires content".to_string(),
            ));
        };

        let original = self.read_original(&path);
        let unmasked = unmask_file_content(&path, original.as_deref(), content, &self.vault);

        if !self.apply_writes {
            let before = original.unwrap_or_default();
            let diff = unified_diff(&path.display().to_string(), &before, &unmasked);
            return Ok(ok(
                RepairToolKind::WriteFile,
                false,
                Some(mask_diff(&diff, &path, &mut self.vault)),
                format!("preview: would write {} byte(s)", unmasked.len()),
            ));
        }

        self.write_text(&path, &unmasked)?;
        Ok(ok(
            RepairToolKind::WriteFile,
            true,
            None,
            format!("wrote {} byte(s) to {}", unmasked.len(), path.display()),
        ))
    }

    fn search_replace(
        &mut self,
        path: Option<&str>,
        old_string: Option<&str>,
        new_string: Option<&str>,
    ) -> Result<RepairToolResult> {
        let path = match self.resolve_path(path, RepairToolKind::SearchReplace) {
            Ok(path) => path,
            Err(result) => return Ok(result),
        };
        let Some(old_string) = old_string else {
            return Ok(failed(
                RepairToolKind::SearchReplace,
                false,
                "search_replace requires old_string".to_string(),
            ));
        };
        let new_string = new_string.unwrap_or("");

        if !path.exists() {
            return Ok(failed(
                RepairToolKind::SearchReplace,
                false,
                format!("file not found: {}", path.display()),
            ));
        }

        let original = self
            .read_original(&path)
            .context("search_replace requires readable file")?;
        let old_unmasked = self.vault.restore_tokens_in_text(old_string);
        let new_unmasked = self.vault.restore_tokens_in_text(new_string);

        let matches = count_occurrences(&original, &old_unmasked);
        if matches == 0 {
            return Ok(failed(
                RepairToolKind::SearchReplace,
                false,
                "old_string not found — copy exact text from read_file (without line numbers)"
                    .to_string(),
            ));
        }
        if matches > 1 {
            return Ok(failed(
                RepairToolKind::SearchReplace,
                false,
                format!("old_string matched {matches} times — include more surrounding context"),
            ));
        }

        let updated = original.replacen(&old_unmasked, &new_unmasked, 1);

        if !self.apply_writes {
            let diff = unified_diff(&path.display().to_string(), &original, &updated);
            return Ok(ok(
                RepairToolKind::SearchReplace,
                false,
                Some(mask_diff(&diff, &path, &mut self.vault)),
                "preview: search_replace ready".to_string(),
            ));
        }

        self.write_text(&path, &updated)?;
        Ok(ok(
            RepairToolKind::SearchReplace,
            true,
            None,
            format!("replaced 1 occurrence in {}", path.display()),
        ))
    }

    fn patch_config(
        &mut self,
        path: Option<&str>,
        key_path: Option<&str>,
        value: Option<&str>,
    ) -> Result<RepairToolResult> {
        let path = match self.resolve_path(path, RepairToolKind::PatchConfig) {
            Ok(path) => path,
            Err(result) => return Ok(result),
        };
        let Some(key_path) = key_path else {
            return Ok(failed(
                RepairToolKind::PatchConfig,
                false,
                "patch_config requires key_path".to_string(),
            ));
        };
        let Some(value) = value else {
            return Ok(failed(
                RepairToolKind::PatchConfig,
                false,
                "patch_config requires value".to_string(),
            ));
        };

        if !path.exists() {
            return Ok(failed(
                RepairToolKind::PatchConfig,
                false,
                format!("file not found: {}", path.display()),
            ));
        }

        let original = self
            .read_original(&path)
            .context("patch_config requires readable file")?;
        let unmasked_value = self.vault.restore_tokens_in_text(value);
        let updated = patch_structured_file(&path, key_path, &unmasked_value)?;

        if !self.apply_writes {
            let diff = unified_diff(&path.display().to_string(), &original, &updated);
            return Ok(ok(
                RepairToolKind::PatchConfig,
                false,
                Some(mask_diff(&diff, &path, &mut self.vault)),
                format!("preview: would set {key_path}"),
            ));
        }

        self.write_text(&path, &updated)?;
        Ok(ok(
            RepairToolKind::PatchConfig,
            true,
            None,
            format!("patched {key_path} in {}", path.display()),
        ))
    }

    fn run_bash(&mut self, command: Option<&str>) -> Result<RepairToolResult> {
        let Some(command) = command else {
            return Ok(failed(
                RepairToolKind::Bash,
                false,
                "bash requires command".to_string(),
            ));
        };

        // The allowlist runs against the restored command — that is what
        // would actually execute — but previews echo the proposal as the
        // model wrote it, so vault tokens never surface real secrets.
        let restored = self.vault.restore_tokens_in_text(command);
        if !bash_command_allowed(&self.runtime_id, &restored) {
            return Ok(failed(
                RepairToolKind::Bash,
                false,
                "command not on repair allowlist".to_string(),
            ));
        }

        if !self.apply_writes {
            return Ok(ok(
                RepairToolKind::Bash,
                false,
                None,
                format!("preview: would run `{command}`"),
            ));
        }

        let output = run_shell(&restored)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            let detail = if stderr.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                stderr.trim().to_string()
            };
            let mut snippet_vault = SecretVault::default();
            let masked = snippet_vault.mask_text(&detail);
            self.vault.absorb(snippet_vault);
            return Ok(failed(
                RepairToolKind::Bash,
                true,
                if masked.trim().is_empty() {
                    format!("command failed: {:?}", output.status.code())
                } else {
                    masked
                },
            ));
        }

        let combined = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("{}\n{}", stdout.trim(), stderr.trim())
        };
        let mut snippet_vault = SecretVault::default();
        let masked = snippet_vault.mask_text(&combined);
        self.vault.absorb(snippet_vault);

        Ok(ok(RepairToolKind::Bash, true, None, masked))
    }

    /// Run a browser tool through the shared CDP session. Everything happens
    /// inside Agent Doctor's isolated automation Chrome, so these calls never
    /// touch the filesystem outside the screenshot PNG and need no approval
    /// card — the chat UI surfaces each step as it runs.
    fn run_browser(
        &mut self,
        kind: RepairToolKind,
        call: &RepairToolCall,
    ) -> Result<RepairToolResult> {
        let applied = !matches!(
            kind,
            RepairToolKind::BrowserSnapshot | RepairToolKind::BrowserGetText
        );
        let Some(tool) = kind.browser_tool_name() else {
            return Ok(failed(kind, false, "not a browser tool".to_string()));
        };
        if !browser_tools_allowed() {
            return Ok(failed(
                kind,
                false,
                "browser tools are switched off for this turn".to_string(),
            ));
        }
        let Some(browser) = self.browser.as_ref() else {
            return Ok(failed(
                kind,
                false,
                "browser tools are not available in this session".to_string(),
            ));
        };
        let args = browser_tool_json(tool, call.browser.as_ref());
        match call_browser_tool(browser, tool, &args) {
            Ok(value) => {
                let mut snippet_vault = SecretVault::default();
                let rendered =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                let masked = snippet_vault.mask_text(&rendered);
                self.vault.absorb(snippet_vault);
                Ok(ok(kind, applied, None, masked))
            }
            Err(error) => Ok(failed(kind, applied, format!("{error:#}"))),
        }
    }

    /// Run one `screen_*` computer-use tool. The click itself happens in the
    /// desktop app over its local socket, so these calls fail with a clear
    /// message when the app is closed. Arguments pass through untouched —
    /// the computer server owns their shape, same as over stdio.
    fn run_computer(&mut self, call: &RepairToolCall) -> Result<RepairToolResult> {
        let Some(computer) = call.computer.as_ref() else {
            return Ok(failed(
                RepairToolKind::Screen,
                false,
                "screen call is missing its tool name".to_string(),
            ));
        };
        let read_only_screen = matches!(
            computer.tool.as_str(),
            "screen_snapshot" | "screen_apps" | "screen_windows" | "screen_state"
        );
        if !browser_tools_allowed() {
            return Ok(failed(
                RepairToolKind::Screen,
                false,
                "screen tools are switched off for this turn".to_string(),
            ));
        }
        if !self.screen_enabled {
            return Ok(failed(
                RepairToolKind::Screen,
                false,
                "screen tools are not available in this session".to_string(),
            ));
        }
        match call_computer_tool(&computer.tool, &computer.arguments) {
            Ok(value) => {
                let mut snippet_vault = SecretVault::default();
                let rendered =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                let masked = snippet_vault.mask_text(&rendered);
                self.vault.absorb(snippet_vault);
                Ok(ok(RepairToolKind::Screen, !read_only_screen, None, masked))
            }
            Err(error) => Ok(failed(
                RepairToolKind::Screen,
                !read_only_screen,
                format!("{error:#}"),
            )),
        }
    }

    fn resolve_path(
        &self,
        path: Option<&str>,
        kind: RepairToolKind,
    ) -> std::result::Result<PathBuf, RepairToolResult> {
        let Some(path) = path else {
            return Err(failed(kind, false, "path is required".to_string()));
        };
        let path = PathBuf::from(path);
        if !path_is_allowed(&path, &self.allowed_paths) {
            return Err(failed(
                kind,
                false,
                format!("path not allowed: {}", path.display()),
            ));
        }
        Ok(path)
    }

    fn read_original(&self, path: &Path) -> Option<String> {
        if path.exists() {
            fs::read_to_string(path).ok()
        } else {
            None
        }
    }

    fn write_text(&mut self, path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        // The allowed-path check ran on un-normalized text (the file may not
        // have existed yet). Now that parents exist, re-check the resolved
        // path so symlinked parents cannot carry the write outside the roots.
        let resolved = path.canonicalize().or_else(|_| {
            path.parent()
                .and_then(|parent| parent.canonicalize().ok())
                .map(|parent| parent.join(path.file_name().unwrap_or_default()))
                .ok_or_else(|| anyhow::anyhow!("failed to resolve {}", path.display()))
        })?;
        if !path_is_allowed(&resolved, &self.allowed_paths) {
            anyhow::bail!("{} resolves outside allowed paths", path.display());
        }
        fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))?;

        if is_env_file(path) {
            let mut refresh_vault = SecretVault::default();
            let _ = mask_config_file(path, content, &mut refresh_vault);
            self.vault.absorb(refresh_vault);
        }
        Ok(())
    }
}

fn format_numbered_lines(masked: &str) -> String {
    masked
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:>6}|{line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.match_indices(needle).count()
}

fn mask_diff(diff: &str, path: &Path, vault: &mut SecretVault) -> String {
    if diff.is_empty() {
        return diff.to_string();
    }
    mask_config_file(path, diff, vault)
}

fn run_shell(command_line: &str) -> Result<std::process::Output> {
    use std::time::Duration;
    let timeout = Duration::from_secs(120);
    #[cfg(unix)]
    let output = agent_doctor_core::exec::run_output("bash", &["-c", command_line], timeout)
        .map_err(|err| anyhow::anyhow!("{err}"))
        .context("failed to start bash")?;

    #[cfg(windows)]
    let output = agent_doctor_core::exec::run_output("cmd", &["/C", command_line], timeout)
        .map_err(|err| anyhow::anyhow!("{err}"))
        .context("failed to start cmd")?;

    Ok(output)
}

fn ok(
    kind: RepairToolKind,
    applied: bool,
    preview_diff: Option<String>,
    masked_output: String,
) -> RepairToolResult {
    RepairToolResult {
        kind,
        success: true,
        masked_output,
        applied,
        preview_diff,
        error: None,
    }
}

fn failed(kind: RepairToolKind, applied: bool, error: String) -> RepairToolResult {
    RepairToolResult {
        kind,
        success: false,
        masked_output: String::new(),
        applied,
        preview_diff: None,
        error: Some(error),
    }
}

/// Mirror [`RepairToolCall`] browser fields into the `agent-doctor-mcp`
/// tool-argument JSON, keeping the same tool contract as the stdio server.
fn browser_tool_json(tool: &str, browser: Option<&BrowserToolArgs>) -> Value {
    let Some(args) = browser else {
        return json!({});
    };
    let mut json_args = json!({});
    let set = |json_args: &mut Value, key: &str, value: &Option<String>| {
        if let Some(text) = value.as_deref().filter(|t| !t.trim().is_empty()) {
            json_args[key] = json!(text);
        }
    };
    match tool {
        "browser_navigate" => set(&mut json_args, "url", &args.url),
        "browser_snapshot" => set(&mut json_args, "selector", &args.target),
        "browser_click" | "browser_fill" => {
            set(&mut json_args, "ref", &args.target);
            if tool == "browser_fill" {
                set(&mut json_args, "text", &args.text);
            }
        }
        "browser_find" => {
            set(&mut json_args, "strategy", &args.strategy);
            set(&mut json_args, "query", &args.query);
            set(&mut json_args, "action", &args.action);
            set(&mut json_args, "text", &args.text);
        }
        "browser_screenshot" => {
            set(&mut json_args, "path", &args.path);
            if args.inline {
                json_args["inline"] = json!(true);
            }
        }
        _ => {}
    }
    json_args
}

pub fn parse_tool_call(name: &str, arguments: &str) -> Result<RepairToolCall> {
    let value: serde_json::Value =
        serde_json::from_str(arguments).context("tool arguments must be JSON object")?;
    let path = value
        .get("path")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let content = value
        .get("content")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let old_string = value
        .get("old_string")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let new_string = value
        .get("new_string")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let key_path = value
        .get("key_path")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let value_field = value
        .get("value")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let command = value
        .get("command")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let pattern = value
        .get("pattern")
        .and_then(|item| item.as_str())
        .map(str::to_string);

    // `ref` wins over `selector` for click/fill, like the MCP server's own
    // target merge; `selector` doubles as the snapshot scope.
    let browser = if name.starts_with("browser_") {
        let target = value
            .get("ref")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .or_else(|| {
                value
                    .get("selector")
                    .and_then(Value::as_str)
                    .filter(|t| !t.trim().is_empty())
            })
            .map(str::to_string);
        let text_field = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        Some(BrowserToolArgs {
            url: text_field("url"),
            text: text_field("text"),
            target,
            strategy: text_field("strategy"),
            query: text_field("query"),
            action: text_field("action"),
            path: text_field("path"),
            inline: value
                .get("inline")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    } else {
        None
    };

    let computer = if name.starts_with("screen_") {
        Some(ComputerToolArgs {
            tool: name.to_string(),
            arguments: value.clone(),
        })
    } else {
        None
    };

    let kind = match name {
        "read_file" => RepairToolKind::Read,
        "list_dir" => RepairToolKind::ListDir,
        "grep_files" => RepairToolKind::GrepFiles,
        "write_file" | "edit_file" => RepairToolKind::WriteFile,
        "search_replace" => RepairToolKind::SearchReplace,
        "patch_config" => RepairToolKind::PatchConfig,
        "bash" => RepairToolKind::Bash,
        "browser_navigate" => RepairToolKind::BrowserNavigate,
        "browser_snapshot" => RepairToolKind::BrowserSnapshot,
        "browser_click" => RepairToolKind::BrowserClick,
        "browser_fill" | "browser_type" => RepairToolKind::BrowserFill,
        "browser_find" => RepairToolKind::BrowserFind,
        "browser_screenshot" => RepairToolKind::BrowserScreenshot,
        "browser_get_text" => RepairToolKind::BrowserGetText,
        "screen_apps" | "screen_open" | "screen_snapshot" | "screen_click" | "screen_type"
        | "screen_press" | "screen_scroll" | "screen_windows" | "screen_state" | "screen_set"
        | "screen_menu" => RepairToolKind::Screen,
        other => bail!("unknown tool: {other}"),
    };

    Ok(RepairToolCall {
        kind,
        path,
        content,
        old_string,
        new_string,
        key_path,
        value: value_field,
        pattern,
        command,
        browser,
        computer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn executor(dir: &TempDir, apply: bool) -> RepairToolExecutor {
        RepairToolExecutor::with_allowed_paths(
            "hermes",
            SecretVault::default(),
            apply,
            vec![dir.path().to_path_buf()],
        )
    }

    #[test]
    fn read_masks_secrets_and_write_restores_them() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(".env");
        let mut file = std::fs::File::create(&path).expect("create");
        writeln!(file, "DEEPSEEK_API_KEY=sk-test-secret-key").expect("write");

        let mut exec = executor(&dir, true);
        let read = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::Read,
                path: Some(path.display().to_string()),
                content: None,
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
                browser: None,
                computer: None,
            })
            .expect("read");
        assert!(read.success);
        assert!(read.masked_output.contains("{{SECRET:"));
        assert!(read.masked_output.contains("|DEEPSEEK_API_KEY="));

        let body = read
            .masked_output
            .lines()
            .map(|line| line.split_once('|').map(|(_, rest)| rest).unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        let write = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::WriteFile,
                path: Some(path.display().to_string()),
                content: Some(body),
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
                browser: None,
                computer: None,
            })
            .expect("write");
        assert!(write.success);
        assert!(write.applied);

        let on_disk = std::fs::read_to_string(&path).expect("read back");
        assert!(on_disk.contains("sk-test-secret-key"));
    }

    #[test]
    fn search_replace_requires_unique_match() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("config.txt");
        std::fs::write(&path, "foo\nfoo\n").expect("write");

        let mut exec = executor(&dir, true);
        let fail = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::SearchReplace,
                path: Some(path.display().to_string()),
                content: None,
                old_string: Some("foo".to_string()),
                new_string: Some("bar".to_string()),
                key_path: None,
                value: None,
                pattern: None,
                command: None,
                browser: None,
                computer: None,
            })
            .expect("replace");
        assert!(!fail.success);
        assert!(fail.error.unwrap().contains("matched 2 times"));
    }

    #[test]
    fn search_replace_preview_includes_diff() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("config.txt");
        std::fs::write(&path, "provider: openai\n").expect("write");

        let mut exec = executor(&dir, false);
        let preview = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::SearchReplace,
                path: Some(path.display().to_string()),
                content: None,
                old_string: Some("openai".to_string()),
                new_string: Some("deepseek".to_string()),
                key_path: None,
                value: None,
                pattern: None,
                command: None,
                browser: None,
                computer: None,
            })
            .expect("preview");
        assert!(preview.success);
        assert!(!preview.applied);
        assert!(preview
            .preview_diff
            .unwrap()
            .contains("+provider: deepseek"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "provider: openai\n"
        );
    }

    #[test]
    fn write_preview_does_not_write() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("notes.txt");
        let mut exec = executor(&dir, false);
        let result = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::WriteFile,
                path: Some(path.display().to_string()),
                content: Some("hello".to_string()),
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
                browser: None,
                computer: None,
            })
            .expect("write preview");
        assert!(result.success);
        assert!(!result.applied);
        assert!(!path.exists());
    }

    #[test]
    fn list_dir_shows_children() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join("config.yaml"), "ok").expect("write");
        let mut exec = executor(&dir, false);
        let result = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::ListDir,
                path: Some(dir.path().display().to_string()),
                content: None,
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
                browser: None,
                computer: None,
            })
            .expect("list");
        assert!(result.success);
        assert!(result.masked_output.contains("config.yaml"));
    }

    #[test]
    fn grep_files_returns_masked_hits() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join("config.yaml"), "provider: openai\n").expect("write");
        let mut exec = executor(&dir, false);
        let result = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::GrepFiles,
                path: Some(dir.path().display().to_string()),
                content: None,
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: Some("provider".to_string()),
                command: None,
                browser: None,
                computer: None,
            })
            .expect("grep");
        assert!(result.success);
        assert!(result.masked_output.contains("config.yaml"));
        assert!(result.masked_output.contains("provider"));
    }

    #[test]
    fn browser_calls_parse_with_ref_winning_over_selector() {
        let parsed = parse_tool_call(
            "browser_navigate",
            r#"{"url":"https://www.baidu.com/s?wd=agent%20doctor"}"#,
        )
        .expect("navigate");
        assert_eq!(parsed.kind, RepairToolKind::BrowserNavigate);
        let browser = parsed.browser.expect("browser args");
        assert_eq!(
            browser.url.as_deref(),
            Some("https://www.baidu.com/s?wd=agent%20doctor")
        );

        let parsed = parse_tool_call("browser_click", r##"{"ref":"@e3","selector":"#submit"}"##)
            .expect("click");
        assert_eq!(parsed.kind, RepairToolKind::BrowserClick);
        assert_eq!(
            parsed.browser.expect("browser args").target.as_deref(),
            Some("@e3")
        );

        let parsed = parse_tool_call(
            "browser_fill",
            r##"{"selector":"#kw","text":"agent doctor"}"##,
        )
        .expect("fill");
        let browser = parsed.browser.expect("browser args");
        assert_eq!(browser.target.as_deref(), Some("#kw"));
        assert_eq!(browser.text.as_deref(), Some("agent doctor"));

        let parsed = parse_tool_call(
            "browser_find",
            r#"{"strategy":"text","query":"登录","action":"click"}"#,
        )
        .expect("find");
        assert_eq!(parsed.kind, RepairToolKind::BrowserFind);
        let browser = parsed.browser.expect("browser args");
        assert_eq!(browser.strategy.as_deref(), Some("text"));
        assert_eq!(browser.query.as_deref(), Some("登录"));
        assert_eq!(browser.action.as_deref(), Some("click"));

        let parsed =
            parse_tool_call("browser_screenshot", r#"{"path":"shot.png"}"#).expect("screenshot");
        assert_eq!(parsed.kind, RepairToolKind::BrowserScreenshot);
        assert_eq!(
            parsed.browser.expect("browser args").path.as_deref(),
            Some("shot.png")
        );
    }

    #[test]
    fn browser_tool_json_keeps_the_mcp_contract() {
        let args = BrowserToolArgs {
            url: Some("https://example.com".to_string()),
            ..BrowserToolArgs::default()
        };
        let json = browser_tool_json("browser_navigate", Some(&args));
        assert_eq!(json["url"], "https://example.com");

        let args = BrowserToolArgs {
            target: Some("@e2".to_string()),
            text: Some("hello".to_string()),
            ..BrowserToolArgs::default()
        };
        let json = browser_tool_json("browser_fill", Some(&args));
        assert_eq!(json["ref"], "@e2");
        assert_eq!(json["text"], "hello");
        assert!(json.get("selector").is_none());

        let json = browser_tool_json("browser_get_text", None);
        assert_eq!(json, serde_json::json!({}));
    }

    #[test]
    fn browser_calls_fail_cleanly_without_a_session() {
        let dir = TempDir::new().expect("tempdir");
        let mut exec = executor(&dir, true);
        let result = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::BrowserNavigate,
                path: None,
                content: None,
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
                browser: Some(BrowserToolArgs {
                    url: Some("https://example.com".to_string()),
                    ..BrowserToolArgs::default()
                }),
                computer: None,
            })
            .expect("browser call");
        assert!(!result.success);
        assert!(result
            .error
            .expect("error")
            .contains("browser tools are not available"));
    }

    #[test]
    fn browser_calls_respect_the_turn_switch() {
        let dir = TempDir::new().expect("tempdir");
        let mut exec = executor(&dir, true);
        let _hold = crate::repair::HoldBrowserToolsOff::new();
        let result = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::BrowserGetText,
                path: None,
                content: None,
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
                browser: None,
                computer: None,
            })
            .expect("browser call");
        assert!(!result.success);
        assert!(result.error.expect("error").contains("switched off"));
    }

    #[test]
    fn screen_calls_pass_tool_name_and_raw_arguments_through() {
        let parsed = parse_tool_call("screen_menu", r#"{"app":"访达","path":["文件","新建"]}"#)
            .expect("menu");
        assert_eq!(parsed.kind, RepairToolKind::Screen);
        let computer = parsed.computer.expect("computer args");
        assert_eq!(computer.tool, "screen_menu");
        assert_eq!(computer.arguments["app"], "访达");
        assert_eq!(computer.arguments["path"][1], "新建");

        let parsed = parse_tool_call("screen_click", r#"{"ref":"@e2"}"#).expect("click");
        assert_eq!(parsed.computer.expect("computer args").tool, "screen_click");
    }

    #[test]
    fn screen_calls_fail_cleanly_when_not_enabled() {
        let dir = TempDir::new().expect("tempdir");
        let mut exec = executor(&dir, true);
        let result = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::Screen,
                path: None,
                content: None,
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
                browser: None,
                computer: Some(ComputerToolArgs {
                    tool: "screen_apps".to_string(),
                    arguments: serde_json::json!({}),
                }),
            })
            .expect("screen call");
        assert!(!result.success);
        assert!(result
            .error
            .expect("error")
            .contains("screen tools are not available"));
    }
}
