use serde_json::Value;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::adapters::util::find_binary;
use crate::exec::{command_for_path, kill_process_tree};

use super::{PromptSessionStatus, MAX_CAPTURE_CHARS, SUMMARY_CHARS};

/// Wall clock for one ask turn.
///
/// Quiet time does not stop the turn. A build that prints nothing keeps
/// running, the same as the agent's own window. `limit` is only the hard stop
/// from the start of the turn, so a stuck process does not run forever.
pub(crate) struct SessionClock {
    started: Instant,
    last_activity: Instant,
    absolute: Duration,
    tool_running: bool,
}

impl SessionClock {
    pub(crate) fn new(limit_sec: u64) -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last_activity: now,
            absolute: Duration::from_secs(limit_sec.max(1)),
            tool_running: false,
        }
    }

    /// The turn produced output, or is waiting on the user. Keep it alive.
    pub(crate) fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    pub(crate) fn set_tool_running(&mut self, running: bool) {
        self.tool_running = running;
    }

    pub(crate) fn expired(&self) -> bool {
        Instant::now() >= self.started + self.absolute
    }

    pub(crate) fn timeout_note(&self, last_tool: &str) -> super::TimeoutNote {
        let now = Instant::now();
        let kind = if now >= self.started + self.absolute {
            "absolute"
        } else if self.tool_running {
            "tool"
        } else {
            "idle"
        };
        super::TimeoutNote {
            kind: kind.to_string(),
            quiet_sec: self.last_activity.elapsed().as_secs(),
            elapsed_sec: self.started.elapsed().as_secs(),
            last_tool: last_tool.trim().to_string(),
        }
    }
}

/// Tool calls that have started and not reported back yet.
#[derive(Default)]
pub(crate) struct ToolWatch {
    open: Vec<String>,
    last_label: String,
}

impl ToolWatch {
    pub(crate) fn start(&mut self, id: &str) {
        if !id.is_empty() && self.open.iter().any(|open| open == id) {
            return;
        }
        self.open.push(id.to_string());
    }

    /// Ends the call with this id. An unknown or empty id ends the oldest one.
    pub(crate) fn finish(&mut self, id: &str) {
        if let Some(pos) = self
            .open
            .iter()
            .position(|open| !id.is_empty() && open == id)
        {
            self.open.remove(pos);
        } else if !self.open.is_empty() {
            self.open.remove(0);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.open.clear();
    }

    pub(crate) fn running(&self) -> bool {
        !self.open.is_empty()
    }

    pub(crate) fn remember(&mut self, label: &str) {
        let line = label.lines().next().unwrap_or("").trim();
        if line.is_empty() {
            return;
        }
        const MAX: usize = 80;
        let count = line.chars().count();
        self.last_label = if count <= MAX {
            line.to_string()
        } else {
            format!("{}…", line.chars().take(MAX).collect::<String>())
        };
    }

    pub(crate) fn last_label(&self) -> &str {
        &self.last_label
    }
}

/// Shown while the model works out its next step and nothing else is live.
pub(crate) const THINKING_AFTER_TOOL: &str = "正在看结果，想下一步…";

/// Streamed reasoning text, turned into the one line the live row shows.
#[derive(Default)]
pub(crate) struct ThinkingLine {
    text: String,
    shown: String,
}

impl ThinkingLine {
    pub(crate) fn reset(&mut self) {
        self.text.clear();
        self.shown.clear();
    }

    /// Returns a new line to show, only when it changed.
    pub(crate) fn push(&mut self, delta: &str) -> Option<String> {
        self.text.push_str(delta);
        let line = thinking_headline(&self.text);
        if line.is_empty() || line == self.shown {
            return None;
        }
        self.shown = line.clone();
        Some(format!("正在思考：{line}"))
    }
}

/// The latest finished `**heading**`, else the latest finished line. A line
/// still being written is skipped so the row does not change on every token.
pub(crate) fn thinking_headline(text: &str) -> String {
    const MAX: usize = 80;
    let mut heading = "";
    let mut rest = text;
    while let Some(open) = rest.find("**") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("**") else { break };
        let inner = after[..close].trim();
        if !inner.is_empty() {
            heading = inner;
        }
        rest = &after[close + 2..];
    }
    let line = if heading.is_empty() {
        let finished = text.rfind('\n').map_or("", |end| &text[..end]);
        finished
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("")
    } else {
        heading
    };
    if line.chars().count() <= MAX {
        line.to_string()
    } else {
        format!("{}…", line.chars().take(MAX).collect::<String>())
    }
}

/// One line for the timeout notice: tool name, plus the command or path.
pub(crate) fn tool_chip(name: &str, detail: &str) -> String {
    let name = name.trim();
    let detail = detail.lines().next().unwrap_or("").trim();
    if name.is_empty() {
        detail.to_string()
    } else if detail.is_empty() {
        name.to_string()
    } else {
        format!("{name}: {detail}")
    }
}

pub(crate) fn push_capped(acc: &Arc<Mutex<String>>, line: &str) {
    if let Ok(mut guard) = acc.lock() {
        if guard.len() >= MAX_CAPTURE_CHARS {
            return;
        }
        let remain = MAX_CAPTURE_CHARS.saturating_sub(guard.len());
        if line.len() <= remain {
            if !guard.is_empty() {
                guard.push('\n');
            }
            guard.push_str(line);
        } else {
            guard.push_str(&line[..remain]);
        }
    }
}

pub(crate) fn combine_output(stdout: &str, stderr: &str) -> String {
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.to_string(),
        (true, false) => stderr.to_string(),
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

/// Status line for a tool chip: a short label, then the command or path on the next line.
pub(crate) fn format_tool_status(label: &str, detail: &str) -> String {
    let label = label.trim();
    let detail = detail.trim();
    if detail.is_empty() {
        format!("调用工具 {label}…")
    } else {
        format!("调用工具 {label}…\n{detail}")
    }
}

fn first_tool_str(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> String {
    for key in keys {
        if let Some(text) = obj.get(*key).and_then(|v| v.as_str()) {
            let text = text.trim();
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }
    String::new()
}

/// Pull a readable command, search, or path out of a tool `input` object.
pub(crate) fn tool_input_detail(input: &Value) -> String {
    if let Some(text) = input.as_str() {
        let text = text.trim();
        if text.starts_with('{') {
            if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                return tool_input_detail(&parsed);
            }
        }
        return clip_tool_detail(text);
    }
    let Some(obj) = input.as_object() else {
        return String::new();
    };
    if obj.is_empty() {
        return String::new();
    }
    let command = first_tool_str(obj, &["command", "cmd", "script"]);
    if !command.is_empty() {
        return clip_tool_detail(&command);
    }
    let pattern = first_tool_str(obj, &["pattern", "query"]);
    let mut path = first_tool_str(obj, &["path", "file_path", "file"]);
    if path == "." || path == "./" {
        path.clear();
    }
    let url = first_tool_str(obj, &["url"]);
    let mut lines = Vec::new();
    if !pattern.is_empty() {
        lines.push(pattern);
    }
    if !path.is_empty() {
        lines.push(path);
    }
    if !url.is_empty() {
        lines.push(url);
    }
    if lines.is_empty() {
        const SKIP: &[&str] = &[
            "content",
            "old_string",
            "new_string",
            "patch",
            "diff",
            "timeout",
            "limit",
            "offset",
        ];
        for (key, value) in obj {
            if SKIP.contains(&key.as_str()) {
                continue;
            }
            let Some(text) = value.as_str() else {
                continue;
            };
            let text = text.trim();
            if text.is_empty() || text.chars().count() > 180 {
                continue;
            }
            lines.push(format!("{key}: {text}"));
            if lines.len() == 4 {
                break;
            }
        }
    }
    if lines.is_empty() {
        let patch = first_tool_str(obj, &["patch", "diff"]);
        if !patch.is_empty() {
            lines.push(patch);
        }
    }
    clip_tool_detail(&lines.join("\n"))
}

fn text_line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn change_preview(old: &str, new: &str) -> String {
    const MAX_LINES: usize = 40;
    let mut lines = Vec::new();
    for line in old.lines().take(MAX_LINES) {
        lines.push(format!("-{line}"));
    }
    for line in new.lines().take(MAX_LINES) {
        lines.push(format!("+{line}"));
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("\n@@diff\n{}", lines.join("\n"))
    }
}

/// File tools carry enough input to show the file and a compact +/− line count.
pub(crate) fn tool_activity_detail(tool: &str, input: &Value) -> String {
    let detail = tool_input_detail(input);
    let Some(obj) = input.as_object() else {
        return detail;
    };
    let key: String = tool
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    if matches!(key.as_str(), "read" | "readfile") && !detail.is_empty() {
        if let Some(start) = obj.get("offset").and_then(Value::as_u64).filter(|n| *n > 0) {
            let limit = obj.get("limit").and_then(Value::as_u64).unwrap_or(1).max(1);
            return format!("{detail} L{start}-{}", start + limit - 1);
        }
    }
    let (additions, deletions, preview) =
        if matches!(key.as_str(), "write" | "writefile" | "createfile") {
            let new = obj.get("content").and_then(Value::as_str).unwrap_or("");
            (text_line_count(new), 0, change_preview("", new))
        } else if matches!(
            key.as_str(),
            "edit" | "multiedit" | "notebookedit" | "filechange"
        ) {
            let new = obj
                .get("new_string")
                .or_else(|| obj.get("newText"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let old = obj
                .get("old_string")
                .or_else(|| obj.get("oldText"))
                .and_then(Value::as_str)
                .unwrap_or("");
            (
                text_line_count(new),
                text_line_count(old),
                change_preview(old, new),
            )
        } else {
            (0, 0, String::new())
        };
    if additions == 0 && deletions == 0 {
        detail
    } else if detail.is_empty() {
        format!("+{additions} -{deletions}{preview}")
    } else {
        format!("{detail}\n+{additions} -{deletions}{preview}")
    }
}

fn clip_tool_detail(text: &str) -> String {
    const MAX: usize = 480;
    let text = text.trim();
    let count = text.chars().count();
    if count <= MAX {
        return text.to_string();
    }
    let clipped: String = text.chars().take(MAX).collect();
    format!("{clipped}…")
}

pub(crate) fn summarize(combined: &str, status: &PromptSessionStatus, runtime: &str) -> String {
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        return format!("{runtime} ask ended ({status:?}) with no captured output");
    }
    if trimmed.chars().count() <= SUMMARY_CHARS {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(SUMMARY_CHARS).collect();
    format!("{truncated}…")
}

pub(crate) fn command_from_cli(bin: &str) -> Command {
    let trimmed = bin.trim();
    let path = Path::new(trimmed);
    let resolved = if path.is_absolute() || trimmed.contains('\\') || trimmed.contains('/') {
        path.to_path_buf()
    } else {
        find_binary(trimmed).unwrap_or_else(|| PathBuf::from(trimmed))
    };
    command_for_path(&resolved)
}

pub(crate) fn force_stop_child(child: &mut Child, pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-P", &pid.to_string()])
            .status();
    }
    kill_process_tree(pid);
    let _ = child.kill();
    let _ = child.wait();
}

/// One-shot CLIs (Hermes / OpenClaw) use `stdin = null` and should exit
/// when both pipes close. If a helper process keeps the parent alive, Ask stays on
/// 「停止」forever — force-stop after a short grace so the UI can accept input again.
pub(crate) fn finish_oneshot_after_pipes_closed(
    child: &mut Child,
    pid: u32,
    stdout_eof: &AtomicBool,
    stderr_eof: &AtomicBool,
    pipes_closed_at: &mut Option<Instant>,
) -> Option<(PromptSessionStatus, Option<i32>)> {
    if !(stdout_eof.load(Ordering::SeqCst) && stderr_eof.load(Ordering::SeqCst)) {
        return None;
    }
    let closed_at = pipes_closed_at.get_or_insert_with(Instant::now);
    match child.try_wait() {
        Ok(Some(wait_status)) => {
            let status = if wait_status.success() {
                PromptSessionStatus::Succeeded
            } else {
                PromptSessionStatus::Failed
            };
            Some((status, wait_status.code()))
        }
        Ok(None) if closed_at.elapsed() >= Duration::from_millis(400) => {
            force_stop_child(child, pid);
            Some((PromptSessionStatus::Succeeded, None))
        }
        _ => None,
    }
}

pub(crate) fn join_reader(handle: thread::JoinHandle<()>, budget: Duration) {
    let start = Instant::now();
    while start.elapsed() < budget {
        if handle.is_finished() {
            let _ = handle.join();
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn is_runtime_stderr_noise(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    // Claude Code SDK warns when ANTHROPIC_MODEL is a gateway id (e.g. DeepSeek)
    // that is not in Claude's built-in list. The request still works; Ask must
    // not show the raw `[claude-code:unrecognized_model] {…}` blob as an error.
    if lower.contains("unrecognized_model") || lower.contains("unrecognized model") {
        return true;
    }
    // Newer Codex strips provider keys from project-local `.codex/config.toml` and
    // logs this on every app-server start. Default workspace cwd is $HOME, so
    // `~/.codex` is rediscovered as project-local while CODEX_HOME is isolated —
    // the warning is expected noise; Ask must not surface it as an error toast.
    if lower.contains("ignored unsupported project-local config") {
        return true;
    }
    // cua-driver prints a TCC auto-launch note and an update banner on stderr.
    // The tool still starts; this is not a failed reply.
    if lower.contains("cua-driver")
        && (lower.contains("is available")
            || lower.contains("update with")
            || lower.contains("release notes")
            || lower.contains("mcp launched")
            || lower.contains("tcc")
            || lower.contains("auto-launching")
            || lower.contains("proxying mcp"))
    {
        return true;
    }
    // A tool whose program file is gone is removed before send. If one still
    // reports that, it is not a failure of this message.
    if (lower.contains("os error 2")
        || lower.contains("no such file")
        || lower.contains("系统找不到"))
        && (lower.contains("mcp") || lower.contains("node_repl"))
    {
        return true;
    }
    // Codex app-server logs this when the client has already finished reading
    // stdout (stdio JSON-RPC shutdown). The turn itself already completed.
    if is_stdio_shutdown_noise(&lower) {
        return true;
    }
    [
        "openai codex",
        "reading additional input from stdin",
        "workdir:",
        "approval:",
        "model:",
        "provider:",
        "session id:",
        "-------",
        "user",
        "assistant",
    ]
    .iter()
    .any(|p| lower.starts_with(p) || lower == *p)
}

/// `EPIPE` / Windows `ERROR_NO_DATA` from a stdio peer that already closed its
/// end of the pipe. Expected while an app-server or LSP-style child shuts down.
fn is_stdio_shutdown_noise(lower: &str) -> bool {
    let closed = lower.contains("broken pipe")
        || lower.contains("os error 32")
        || lower.contains("os error 232")
        || lower.contains("pipe is being closed")
        || lower.contains("pipe has been ended");
    if !closed {
        return false;
    }
    lower.contains("stdout")
        || lower.contains("stdin")
        || lower.contains("stdio")
        || lower.contains("codex_app_server_transport")
}

pub(crate) fn strip_runtime_stderr_noise(raw: &str) -> String {
    raw.lines()
        .filter(|line| !is_runtime_stderr_noise(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Map Codex app-server / policy jargon into short Chinese UI copy.
pub(crate) fn humanize_runtime_error(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return t.to_string();
    }
    let lower = t.to_ascii_lowercase();

    if lower.contains("unsupported call") && lower.contains("apply_patch") {
        return "当前的 Codex 不支持这种改文件方式，它会换一种方式继续。".into();
    }
    if lower.contains("apply_patch") && (lower.contains("hunk") || lower.contains("verification")) {
        return "写文件补丁格式不正确：每一段必须以 `*** Add File: 路径` / `*** Update File: 路径` / `*** Delete File: 路径` 开头，不能把文件内容写在标题行。模型应修正补丁后重试。".into();
    }
    if lower.contains("rm -f style")
        || (lower.contains("rejected") && lower.contains("safer approach"))
    {
        return "这一步要清掉临时文件才能继续。点「允许」。".into();
    }
    if lower.contains("unlesstrusted") || lower.contains("unless trusted") {
        return "当前审批策略为「不信任除非已信任」(UnlessTrusted)，不能使用 require_escalated 提升权限。请改用普通命令，或关闭 elevated/全自动后重试。".into();
    }
    if lower.contains("require_escalated") || lower.contains("escalated permissions") {
        return "模型请求了提升权限，但当前策略不允许。请用普通工具操作，或在聊天里通过「允许 / 拒绝」确认。".into();
    }
    if lower.contains("unknown variant") && lower.contains("approval") {
        return format!("审批策略参数不被本地 Codex 接受。详情：{t}");
    }
    if lower.contains("unknown variant")
        && (lower.contains("sandbox") || lower.contains("workspace"))
    {
        return format!("沙箱参数与本地 Codex 版本不匹配。详情：{t}");
    }
    if lower.contains("invalid request") {
        return format!("Codex 请求无效（多为协议字段与 CLI 版本不一致）。详情：{t}");
    }
    if lower.contains("codex thread did not open") {
        return "Codex 没能开始这一轮。请再发一次。".into();
    }
    t.to_string()
}

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_missing_tool_startup_on_every_runtime() {
        assert!(is_runtime_stderr_noise(
            "MCP client for `node_repl` failed to start: MCP startup failed: No such file or directory (os error 2)"
        ));
        assert!(!is_runtime_stderr_noise("provider rejected the key"));
        assert!(is_runtime_stderr_noise(
            "cua-driver: mcp launched without CuaDriver.app's TCC grants; auto-launching the daemon"
        ));
        assert!(is_runtime_stderr_noise(
            "cua-driver v0.30.4 is available (you have v0.28.2)."
        ));
        assert!(!is_runtime_stderr_noise(
            "cua-driver failed: permission denied"
        ));
        assert!(is_runtime_stderr_noise(
            "2026-09-28T09:19:04.710092Z ERROR codex_app_server_transport::transport::stdio: Failed to write to stdout: Broken pipe (os error 32)"
        ));
        assert!(is_runtime_stderr_noise(
            "Failed to write to stdout: The pipe is being closed. (os error 232)"
        ));
        assert!(!is_runtime_stderr_noise(
            "Failed to write to stdout: permission denied (os error 13)"
        ));
    }

    #[test]
    fn humanizes_unsupported_apply_patch() {
        let msg = humanize_runtime_error(
            "2026-10-10T14:13:22.908636Z ERROR codex_core::tools::router: error=unsupported call: apply_patch",
        );
        assert!(msg.contains("不支持"));
        assert!(!msg.contains("codex_core"));
    }

    #[test]
    fn humanizes_apply_patch_hunk_error() {
        let msg = humanize_runtime_error(
            "apply_patch verification failed: invalid hunk at line 3, 'hello from codex' is not a valid hunk header",
        );
        assert!(msg.contains("补丁") || msg.contains("Add File"));
    }

    #[test]
    fn humanizes_forced_delete_rejection() {
        let msg = humanize_runtime_error(
            "Rejected(\"`/bin/zsh -c \\\"rm -rf /tmp/clips\\\"` rejected: rm -f style commands are not permitted. Use a safer approach\")",
        );
        assert!(msg.contains("允许"));
        assert!(!msg.contains("rm -f style"));
    }

    #[test]
    fn humanizes_unless_trusted_escalate() {
        let msg = humanize_runtime_error(
            "approval policy is UnlessTrusted; reject command — you cannot ask for escalated permissions",
        );
        assert!(msg.contains("UnlessTrusted") || msg.contains("提升权限"));
        assert!(!msg.starts_with("approval policy is"));
    }

    #[test]
    fn silence_does_not_stop_the_turn() {
        let mut clock = SessionClock::new(600);
        clock.last_activity -= Duration::from_secs(10_000);
        clock.set_tool_running(true);
        assert!(!clock.expired());
    }

    #[test]
    fn the_turn_stops_at_its_time_limit() {
        let mut clock = SessionClock::new(600);
        clock.started -= Duration::from_secs(600);
        clock.set_tool_running(true);
        clock.touch();
        assert!(clock.expired());
        assert_eq!(clock.timeout_note("npm test").kind, "absolute");
    }

    #[test]
    fn timeout_note_names_the_open_tool_or_the_last_one() {
        let mut clock = SessionClock::new(600);
        clock.last_activity -= Duration::from_secs(700);
        let idle = clock.timeout_note("Read: src/login.ts");
        assert_eq!(idle.kind, "idle");
        assert!(idle.quiet_sec >= 700);
        assert_eq!(idle.last_tool, "Read: src/login.ts");

        clock.set_tool_running(true);
        assert_eq!(clock.timeout_note("Bash: npm test").kind, "tool");
    }

    #[test]
    fn thinking_line_shows_finished_headings_only() {
        let mut line = ThinkingLine::default();
        assert_eq!(line.push("**Listing fi"), None);
        assert_eq!(
            line.push("les**\n\nI will run"),
            Some("正在思考：Listing files".into())
        );
        assert_eq!(line.push(" ls next."), None);
        assert_eq!(
            line.push("\n\n**Reading notes**"),
            Some("正在思考：Reading notes".into())
        );

        let mut plain = ThinkingLine::default();
        assert_eq!(plain.push("Check the folder"), None);
        assert_eq!(
            plain.push(" first.\nThen"),
            Some("正在思考：Check the folder first.".into())
        );
    }

    #[test]
    fn tool_watch_pairs_results_by_id() {
        let mut watch = ToolWatch::default();
        watch.start("a");
        watch.start("b");
        watch.finish("b");
        assert!(watch.running());
        watch.finish("");
        assert!(!watch.running());
        watch.finish("missing");
        assert!(!watch.running());
    }

    #[test]
    fn tool_status_keeps_command_on_its_own_line() {
        let input = serde_json::json!({"command": "git status", "timeout": 30});
        let detail = tool_input_detail(&input);
        assert_eq!(detail, "git status");
        assert_eq!(
            format_tool_status("终端", &detail),
            "调用工具 终端…\ngit status"
        );
        assert_eq!(
            tool_input_detail(&serde_json::json!({"path": "src/chat.css"})),
            "src/chat.css"
        );
        assert_eq!(
            tool_input_detail(&serde_json::json!({
                "pattern": "fn main",
                "path": ".",
                "target": "content"
            })),
            "fn main"
        );
    }

    #[test]
    fn file_tool_detail_includes_line_changes() {
        assert_eq!(
            tool_activity_detail(
                "Read",
                &serde_json::json!({"file_path": "src/chat.ts", "offset": 330, "limit": 80})
            ),
            "src/chat.ts L330-409"
        );
        assert_eq!(
            tool_activity_detail(
                "Edit",
                &serde_json::json!({
                    "file_path": "src/chat.ts",
                    "old_string": "one\ntwo",
                    "new_string": "one\nthree\nfour"
                })
            ),
            "src/chat.ts\n+3 -2\n@@diff\n-one\n-two\n+one\n+three\n+four"
        );
        assert_eq!(
            tool_activity_detail(
                "Write",
                &serde_json::json!({"path": "src/new.ts", "content": "a\nb"})
            ),
            "src/new.ts\n+2 -0\n@@diff\n+a\n+b"
        );
    }

    #[test]
    fn humanizes_unknown_sandbox_variant() {
        let msg = humanize_runtime_error(
            "Invalid request: unknown variant `workspaceWrite`, expected one of `read-only`",
        );
        assert!(msg.contains("沙箱") || msg.contains("Codex"));
    }
}
